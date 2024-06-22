use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use std::os::unix::fs::PermissionsExt;
use std::str;

async fn handle_client(mut socket: tokio::net::TcpStream, root_folder: PathBuf, valid_user: &str, valid_pass: &str) {
    let mut buffer = vec![0; 1024];
    if let Ok(n) = socket.read(&mut buffer).await {
        if n == 0 {
            return;
        }

        let request = String::from_utf8_lossy(&buffer[..n]);
        let mut lines = request.lines();
        if let Some(request_line) = lines.next() {
            let parts: Vec<&str> = request_line.split_whitespace().collect();
            if parts.len() == 3 {
                let method = parts[0];
                let path = parts[1];

                // Verificare autentificare
                let mut authenticated = false;
                for line in &mut lines {
                    if line.starts_with("Authorization: Basic ") {
                        let base64_credentials = &line["Authorization: Basic ".len()..];
                        if let Ok(decoded) = base64_decode(base64_credentials) {
                            if let Ok(credentials) = String::from_utf8(decoded) {
                                let mut creds = credentials.splitn(2, ':');
                                if let (Some(user), Some(pass)) = (creds.next(), creds.next()) {
                                    if user == valid_user && pass == valid_pass {
                                        authenticated = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if !authenticated {
                    let response = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic\r\n\r\n";
                    socket.write_all(response.as_bytes()).await.unwrap();
                    return;
                }

                let full_path = root_folder.join(path.trim_start_matches('/'));

                let response = if method == "GET" {
                    handle_get_request(&full_path).await
                } else {
                    handle_post_request(&full_path, &mut lines.collect::<Vec<&str>>()).await
                };

                socket.write_all(response.as_bytes()).await.unwrap();
            }
        }
    }
}

async fn handle_get_request(path: &Path) -> String {
    if path.is_file() {
        match tokio::fs::read(path).await {
            Ok(contents) => {
                let mime_type = guess_mime_type(path);
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\n\r\n{}",
                    mime_type,
                    String::from_utf8_lossy(&contents)
                )
            }
            Err(_) => "HTTP/1.1 403 Forbidden\r\n\r\n403 Forbidden".to_string(),
        }
    } else if path.is_dir() {
        let mut entries = tokio::fs::read_dir(path).await.unwrap();
        let mut body = String::new();
        body.push_str("<html><h1>Directory Listing</h1><ul>");
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().into_string().unwrap();
            body.push_str(&format!("<li><a href=\"/{0}\">{0}</a></li>", name));
        }
        body.push_str("</ul></html>");
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}",
            body
        )
    } else {
        "HTTP/1.1 404 Not Found\r\n\r\n404 Not Found".to_string()
    }
}

async fn handle_post_request(path: &Path, headers: &[&str]) -> String {
    if path.starts_with("/scripts") {
        match tokio::fs::metadata(path).await {
            Ok(metadata) => {
                if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                    let mut command = Command::new(path);
                    for header in headers {
                        if let Some((key, value)) = header.split_once(": ") {
                            command.env(key, value);
                        }
                    }
                    match command.output().await {
                        Ok(output) => {
                            if output.status.success() {
                                format!(
                                    "HTTP/1.1 200 OK\r\n\r\n{}",
                                    String::from_utf8_lossy(&output.stdout)
                                )
                            } else {
                                format!(
                                    "HTTP/1.1 500 Internal Server Error\r\n\r\n{}",
                                    String::from_utf8_lossy(&output.stderr)
                                )
                            }
                        }
                        Err(_) => "HTTP/1.1 500 Internal Server Error\r\n\r\n500 Internal Server Error".to_string(),
                    }
                } else {
                    "HTTP/1.1 403 Forbidden\r\n\r\n403 Forbidden".to_string()
                }
            }
            Err(_) => "HTTP/1.1 403 Forbidden\r\n\r\n403 Forbidden".to_string(),
        }
    } else {
        "HTTP/1.1 403 Forbidden\r\n\r\n403 Forbidden".to_string()
    }
}

// Implementare manuală a decodării base64
fn base64_decode(encoded: &str) -> Result<Vec<u8>, ()> {
    let bytes = encoded.as_bytes();
    let mut buffer = Vec::new();
    let mut padding = 0;

    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        let mut bits = 0;

        for &byte in chunk {
            acc <<= 6;
            match byte {
                b'A'..=b'Z' => acc |= (byte - b'A') as u32,
                b'a'..=b'z' => acc |= (byte - b'a' + 26) as u32,
                b'0'..=b'9' => acc |= (byte - b'0' + 52) as u32,
                b'+' => acc |= 62,
                b'/' => acc |= 63,
                b'=' => {
                    acc >>= 6;
                    padding += 1;
                }
                _ => return Err(()),
            }
            bits += 6;
        }

        buffer.extend_from_slice(&acc.to_be_bytes()[1..1 + (bits - padding * 6) / 8]);
    }

    Ok(buffer)
}

fn guess_mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("png") => "image/png",
        Some("jpeg") | Some("jpg") => "image/jpeg",
        Some("zip") => "application/zip",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!("Usage: {} PORT ROOT_FOLDER USERNAME PASSWORD", args[0]);
        return;
    }

    let port: u16 = args[1].parse().expect("Invalid port number");
    let root_folder = PathBuf::from(&args[2]);
    let username = args[3].clone();
    let password = args[4].clone();

    println!("Root folder: {:?}", root_folder.canonicalize().unwrap());
    println!("Server listening on 0.0.0.0:{}", port);

    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();

    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let root_folder = root_folder.clone();
        let username = username.clone();
        let password = password.clone();
        tokio::spawn(async move {
            handle_client(socket, root_folder, &username, &password).await;
        });
    }
}
