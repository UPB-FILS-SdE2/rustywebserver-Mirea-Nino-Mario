use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::env;
use std::path::{Path, PathBuf};
use std::fs;
use tokio::process::Command;
use std::os::unix::fs::PermissionsExt;

async fn handle_client(mut socket: tokio::net::TcpStream, root_folder: PathBuf) {
    let mut buffer = [0; 1024];
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
        match fs::read(path) {
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
        let entries = fs::read_dir(path).unwrap();
        let mut body = String::new();
        body.push_str("<html><h1>Directory Listing</h1><ul>");
        for entry in entries {
            let entry = entry.unwrap();
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
    if path.starts_with("/scripts") && path.is_file() && path.metadata().unwrap().permissions().mode() & 0o111 != 0 {
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
    if args.len() != 3 {
        eprintln!("Usage: {} PORT ROOT_FOLDER", args[0]);
        return;
    }

    let port: u16 = args[1].parse().expect("Invalid port number");
    let root_folder = PathBuf::from(&args[2]);

    println!("Root folder: {:?}", root_folder.canonicalize().unwrap());
    println!("Server listening on 0.0.0.0:{}", port);

    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();

    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let root_folder = root_folder.clone();
        tokio::spawn(async move {
            handle_client(socket, root_folder).await;
        });
    }
}
