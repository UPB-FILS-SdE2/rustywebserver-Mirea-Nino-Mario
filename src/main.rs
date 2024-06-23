use tokio::net::{TcpListener, TcpStream};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;

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
    let (request_line, headers, message) = parse_request(&request);
    let (method, path, _) = process_request_line(&request_line);

    let client_ip = stream.peer_addr()?.ip().to_string();

    if path.starts_with("/scripts/") {
        handle_script(&mut stream, &root, method, path, &headers, &message, &client_ip).await?;
    } else if method == "GET" {
        handle_get(&mut stream, &root, path, &client_ip).await?;
    } else {
        log_request(&client_ip, path, 501, "Not Implemented");
        send_response(&mut stream, 501, "Not Implemented", "text/plain; charset=utf-8", "Method not implemented").await?;
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
    let file_path = format!("{}{}", root, path);
    let full_path = Path::new(&file_path);

    if !full_path.starts_with(root) {
        log_request(client_ip, path, 403, "Forbidden");
        send_response(stream, 403, "Forbidden", "text/plain; charset=utf-8", "Access denied").await?;
        return Ok(());
    }

    if full_path.is_dir() {
        handle_directory_listing(stream, full_path, path, client_ip).await?;
    } else if full_path.is_file() {
        let content = fs::read(full_path).await?;
        let content_type = get_content_type(full_path);
        log_request(client_ip, path, 200, "OK");
        send_binary_response(stream, 200, "OK", &content_type, &content).await?;
    } else {
        log_request(client_ip, path, 404, "Not Found");
        send_response(stream, 404, "Not Found", "text/plain; charset=utf-8", "File not found").await?;
    }

    Ok(())
}

async fn send_binary_response(stream: &mut TcpStream, status_code: u32, status: &str, content_type: &str, content: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: closed\r\n\r\n",
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

async fn handle_script(stream: &mut TcpStream, root: &str, method: &str, path: &str, headers: &HashMap<String, String>, message: &str, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let script_path = format!("{}{}", root, path);
    let full_path = Path::new(&script_path);

    if !full_path.starts_with(root) || !path.starts_with("/scripts/") {
        log_request(client_ip, path, 403, "Forbidden");
        send_response(stream, 403, "Forbidden", "text/plain; charset=utf-8", "Access denied").await?;
        return Ok(());
    }

    if !full_path.exists() || !full_path.is_file() {
        log_request(client_ip, path, 404, "Not Found");
        send_response(stream, 404, "Not Found", "text/plain; charset=utf-8", "Script not found").await?;
        return Ok(());
    }

    let mut command = Command::new(script_path);
    command.current_dir(PathBuf::from(root).join("scripts"));

    // Set environment variables
    for (key, value) in headers {
        command.env(key, value);
    }
    command.env("Method", method);
    command.env("Path", path);

    // Handle query string
    if let Some(query_string) = path.split('?').nth(1) {
        for pair in query_string.split('&') {
            let mut parts = pair.split('=');
            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                command.env(format!("Query_{}", key), value);
            }
        }
    }

    // Handle POST data
    if method == "POST" {
        command.env("CONTENT_LENGTH", message.len().to_string());
        command.env("CONTENT_TYPE", headers.get("Content-Type").unwrap_or(&String::new()));
    }

    let output = if method == "POST" {
        command.arg(message).output().await?
    } else {
        command.output().await?
    };

    if output.status.success() {
        log_request(client_ip, path, 200, "OK");
        send_response(stream, 200, "OK", "text/plain; charset=utf-8", &String::from_utf8_lossy(&output.stdout)).await?;
    } else {
        log_request(client_ip, path, 500, "Internal Server Error");
        send_response(stream, 500, "Internal Server Error", "text/plain; charset=utf-8", &String::from_utf8_lossy(&output.stderr)).await?;
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

fn log_request(client_ip: &str, path: impl AsRef<Path>, status_code: u32, status_text: &str) {
    println!("GET {} {} -> {} ({})", client_ip, path.as_ref().display(), status_code, status_text);
}