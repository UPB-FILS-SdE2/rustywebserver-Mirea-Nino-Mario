use tokio::net::{TcpListener, TcpStream};
use tokio::fs;
use tokio::process::Command;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <PORT> <ROOT_FOLDER>", args[0]);
        std::process::exit(1);
    }

    let port = &args[1];
    let root_folder = &args[2];

    println!("Root folder: {}", fs::canonicalize(root_folder).await?.display());
    println!("Server listening on 0.0.0.0:{}", port);

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    let root = Arc::new(root_folder.to_string());

    loop {
        let (stream, _) = listener.accept().await?;
        let root = Arc::clone(&root);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, root).await {
                eprintln!("Error handling connection: {}", e);
            }
        });
    }
    
}

async fn handle_connection(mut stream: TcpStream, root: Arc<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = [0; 8192];
    let size = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let (request_line, headers, _) = parse_request(&request);
    let (_, path, _) = process_request_line(&request_line);

    let client_ip = stream.peer_addr()?.ip().to_string();

    if path.starts_with("/scripts/") {
        handle_script(&mut stream, &root, &path, &headers, &client_ip).await?;
    } else {
        handle_get(&mut stream, &root, &path, &client_ip).await?;
    }

    Ok(())
}

fn parse_request(request: &str) -> (String, HashMap<String, String>, String) {
    let mut parts = request.splitn(2, "\r\n\r\n");
    let header_part = parts.next().unwrap_or("");
    let message = parts.next().unwrap_or("").to_string();

    let mut headers = header_part.lines();
    let request_line = headers.next().unwrap_or("").to_string();

    let mut header_map = HashMap::new();
    for header in headers {
        let mut parts = header.splitn(2, ": ");
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            header_map.insert(key.to_string(), value.to_string());
        }
    }

    (request_line, header_map, message)
}

fn process_request_line(request_line: &str) -> (&str, &str, &str) {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    (method, path, version)
}

async fn handle_get(stream: &mut TcpStream, root: &str, path: &str, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root_path = PathBuf::from(root);
    let requested_path = root_path.join(path.trim_start_matches('/'));
    
    // Normalize both paths to compare them properly
    let normalized_requested_path = match fs::canonicalize(&requested_path).await {
        Ok(p) => p,
        Err(_) => {
            log_request(client_ip, path, 404, "Not Found");
            send_response(stream, 404, "Not Found", "text/html; charset=utf-8", "<html>404 Not Found</html>").await?;
            return Ok(());
        }
    };
    
    let normalized_root_path = fs::canonicalize(&root_path).await?;

    // Check if the normalized requested path starts with the normalized root path
    if !normalized_requested_path.starts_with(&normalized_root_path) {
        log_request(client_ip, path, 403, "Forbidden");
        send_response(stream, 403, "Forbidden", "text/html; charset=utf-8", "<html>403 Forbidden</html>").await?;
        return Ok(());
    }

    match fs::metadata(&normalized_requested_path).await {
        Ok(metadata) => {
            if metadata.is_dir() {
                handle_directory_listing(stream, &normalized_requested_path, path, client_ip).await?;
            } else if metadata.is_file() {
                match fs::read(&normalized_requested_path).await {
                    Ok(content) => {
                        let content_type = get_content_type(&normalized_requested_path);
                        log_request(client_ip, path, 200, "OK");
                        send_binary_response(stream, 200, "OK", &content_type, &content).await?;
                    },
                    Err(e) => {
                        eprintln!("Error reading file: {:?}", e);
                        log_request(client_ip, path, 403, "Forbidden");
                        send_response(stream, 403, "Forbidden", "text/html; charset=utf-8", "<html>403 Forbidden</html>").await?;
                    }
                }
            } else {
                log_request(client_ip, path, 404, "Not Found");
                send_response(stream, 404, "Not Found", "text/html; charset=utf-8", "<html>404 Not Found</html>").await?;
            }
        },
        Err(e) => {
            eprintln!("Error getting metadata: {:?}", e);
            log_request(client_ip, path, 404, "Not Found");
            send_response(stream, 404, "Not Found", "text/html; charset=utf-8", "<html>404 Not Found</html>").await?;
        }
    }

    Ok(())
}

async fn send_binary_response(stream: &mut TcpStream, status_code: u32, status: &str, content_type: &str, content: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code, status, content_type, content.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(content).await?;
    Ok(())
}

async fn handle_directory_listing(stream: &mut TcpStream, full_path: &Path, display_path: &str, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut html = String::from("<html><h1>Directory Listing</h1><ul>");

    html.push_str("<li><a href=\"..\">..</a></li>");

    let mut entries = fs::read_dir(full_path).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name();
        if let Some(name) = file_name.to_str() {
            html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", name, name));
        }
    }

    html.push_str("</ul></html>");

    log_request(client_ip, display_path, 200, "OK");
    send_response(stream, 200, "OK", "text/html; charset=utf-8", &html).await?;

    Ok(())
}

async fn handle_script(stream: &mut TcpStream, root: &str, path: &str, headers: &HashMap<String, String>, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let script_path = format!("{}/scripts/{}", root, path.trim_start_matches("/scripts/"));
    let script_path = Path::new(&script_path);

    if !script_path.exists() || !script_path.is_file() {
        log_request(client_ip, path, 404, "Not Found");
        send_response(stream, 404, "Not Found", "text/html; charset=utf-8", "<html>404 Not Found</html>").await?;
        return Ok(());
    }

    let mut command = Command::new(script_path);
    command.env_clear()
           .envs(headers)
           .env("METHOD", "GET")
           .env("PATH", path)
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());
    
    match command.output().await {
        Ok(output) => {
            if output.status.success() {
                // Handle successful script execution (unchanged)
                // ...
            } else {
                log_request(client_ip, path, 500, "Internal Server Error");
                send_response(
                    stream,
                    500,
                    "Internal Server Error",
                    "text/html; charset=utf-8",
                    "<html>500 Internal Server Error</html>"
                ).await?;
            }
        },
        Err(_) => {
            log_request(client_ip, path, 500, "Internal Server Error");
            send_response(
                stream,
                500,
                "Internal Server Error",
                "text/html; charset=utf-8",
                "<html>500 Internal Server Error</html>"
            ).await?;
        }
    }

    Ok(())
}



fn get_content_type(path: &Path) -> String {
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("txt") => "text/plain; charset=utf-8".to_string(),
        Some("html") => "text/html; charset=utf-8".to_string(),
        Some("css") => "text/css; charset=utf-8".to_string(),
        Some("js") => "text/javascript; charset=utf-8".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("png") => "image/png".to_string(),
        Some("zip") => "application/zip".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

async fn send_response(stream: &mut TcpStream, status_code: u32, status: &str, content_type: &str, message: &str) -> Result<(), Box<dyn std::error::Error>> {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_code, status, content_type, message.len(), message
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn log_request(client_ip: &str, path: &str, status_code: u32, status_text: &str) {
    println!("GET {} {} -> {} ({})", client_ip, path, status_code, status_text);
}