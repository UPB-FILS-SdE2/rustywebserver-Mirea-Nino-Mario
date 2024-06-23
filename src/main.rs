use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} PORT ROOT_FOLDER", args[0]);
        return;
    }

    let port: u16 = match args[1].parse() {
        Ok(port) => port,
        Err(_) => {
            eprintln!("Invalid port number: {}", args[1]);
            return;
        }
    };

    let root_folder = PathBuf::from(&args[2]);

    //println!("Root folder: {:?}", root_folder.canonicalize().unwrap());

    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => {
            //println!("Server listening on 0.0.0.0:{}", port);
            listener
        }
        Err(e) => {
            eprintln!("Failed to bind to port {}: {}", port, e);
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((socket, _)) => {
                //println!("Accepted new connection");
                let root_folder = root_folder.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(socket, root_folder).await {
                        eprintln!("Failed to handle client: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("Failed to accept connection: {}", e),
        }
    }
}

async fn handle_client(mut socket: tokio::net::TcpStream, root_folder: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![0; 1024];
    let n = socket.read(&mut buffer).await?;
    if n == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..n]);
    let mut lines = request.lines();
    if let Some(request_line) = lines.next() {
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() == 3 {
            let method = parts[0];
            let path = parts[1];

            let response = if method == "GET" {
                handle_get_request(&root_folder, path).await
            } else if method == "POST" && path == "/subsets" {
                handle_post_subsets_request(&mut socket, &mut lines.collect::<Vec<&str>>()).await
            } else {
                handle_post_request(&root_folder, path, &mut lines.collect::<Vec<&str>>()).await
            };

            socket.write_all(response.as_bytes()).await?;
        }
    }
    Ok(())
}

async fn handle_get_request(root_folder: &Path, path: &str) -> String {
    let full_path = root_folder.join(path.trim_start_matches('/'));

    match tokio::fs::metadata(&full_path).await {
        Ok(file_metadata) => {
            if file_metadata.is_file() {
                match tokio::fs::read(&full_path).await {
                    Ok(contents) => {
                        let mime_type = guess_mime_type(&full_path);
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            mime_type,
                            contents.len(),
                            String::from_utf8_lossy(&contents)
                        )
                    }
                    Err(_) => {
                        "HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\n\r\n500 Internal Server Error".to_string()
                    }
                }
            } else if file_metadata.is_dir() {
                match tokio::fs::read_dir(&full_path).await {
                    Ok(mut entries) => {
                        let mut body = String::new();
                        body.push_str("<html><h1>Directory Listing</h1><ul>");
                        while let Some(entry) = entries.next_entry().await.unwrap_or_else(|_| None) {
                            let name = entry.file_name().into_string().unwrap_or_default();
                            body.push_str(&format!("<li><a href=\"/{0}\">{0}</a></li>", name));
                        }
                        body.push_str("</ul></html>");
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    }
                    Err(_) => {
                        "HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\n\r\n500 Internal Server Error".to_string()
                    }
                }
            } else {
                "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n404 Not Found".to_string()
            }
        }
        Err(_) => {
            "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n404 Not Found".to_string()
        }
    }
}




async fn handle_post_subsets_request(_socket: &mut tokio::net::TcpStream, headers: &[&str]) -> String {
    let mut body = String::new();
    for header in headers {
        body.push_str(header);
    }

    let request: HashMap<String, String> = match parse_json(&body) {
        Ok(req) => req,
        Err(_) => return "HTTP/1.1 400 Bad Request\r\n\r\nInvalid JSON".to_string(),
    };

    let words: Vec<String> = match request.get("words") {
        Some(words_str) => words_str.split(',').map(|s| s.trim().to_string()).collect(),
        None => return "HTTP/1.1 400 Bad Request\r\n\r\nMissing 'words' field".to_string(),
    };

    let k: usize = match request.get("k") {
        Some(k_str) => match k_str.parse() {
            Ok(k) => k,
            Err(_) => return "HTTP/1.1 400 Bad Request\r\n\r\nInvalid 'k' field".to_string(),
        },
        None => return "HTTP/1.1 400 Bad Request\r\n\r\nMissing 'k' field".to_string(),
    };

    let subsets = find_optimal_solutions(words, k);
    let response = format!("{{\"subsets\": {:?}}}", subsets);
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
        response
    )
}

async fn handle_post_request(root_folder: &Path, path: &str, headers: &[&str]) -> String {
    let full_path = root_folder.join(path.trim_start_matches('/'));
    
    if full_path.starts_with("/scripts") {
        match tokio::fs::metadata(&full_path).await {
            Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => {
                let mut command = tokio::process::Command::new(&full_path);
                for header in headers {
                    if let Some((key, value)) = header.split_once(": ") {
                        command.env(key, value);
                    }
                }
                match command.output().await {
                    Ok(output) => {
                        let status_code = if output.status.success() {
                            "200 OK"
                        } else {
                            "500 Internal Server Error"
                        };
                        let response_body = String::from_utf8_lossy(&output.stdout);
                        format!(
                            "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            status_code,
                            response_body.len(),
                            response_body
                        )
                    }
                    Err(_) => "HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\n\r\n500 Internal Server Error".to_string(),
                }
            }
            _ => "HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n403 Forbidden".to_string(),
        }
    } else {
        "HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n403 Forbidden".to_string()
    }
}


fn guess_mime_type<P: AsRef<Path>>(path: P) -> &'static str {
    match path.as_ref().extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        _ => "application/octet-stream",
    }
}

fn find_optimal_solutions(words: Vec<String>, k: usize) -> Vec<Vec<String>> {
    fn combinations<T: Clone>(v: &[T], k: usize) -> Vec<Vec<T>> {
        let mut result = Vec::new();
        let n = v.len();
        if k > n {
            return result;
        }
        let mut indices: Vec<usize> = (0..k).collect();
        loop {
            result.push(indices.iter().map(|&i| v[i].clone()).collect());
            let mut i = k;
            while i > 0 {
                i -= 1;
                if indices[i] != i + n - k {
                    break;
                }
            }
            if i == 0 {
                break;
            }
            indices[i] += 1;
            for j in i + 1..k {
                indices[j] = indices[j - 1] + 1;
            }
        }
        result
    }

    let all_combinations = combinations(&words, k);
    let min_value = all_combinations
        .iter()
        .map(|comb| comb.iter().map(|word| word.chars().map(|c| c as u32 - 'a' as u32 + 1).sum::<u32>()).sum::<u32>())
        .min()
        .unwrap();

    all_combinations
        .into_iter()
        .filter(|comb| comb.iter().map(|word| word.chars().map(|c| c as u32 - 'a' as u32 + 1).sum::<u32>()).sum::<u32>() == min_value)
        .collect()
}

fn parse_json(body: &str) -> Result<HashMap<String, String>, ()> {
    let mut map = HashMap::new();
    if body.starts_with('{') && body.ends_with('}') {
        let body = &body[1..body.len()-1];
        for pair in body.split(',') {
            if let Some((key, value)) = pair.split_once(':') {
                let key = key.trim_matches(|c| c == '"' || c == ' ');
                let value = value.trim_matches(|c| c == '"' || c == ' ');
                map.insert(key.to_string(), value.to_string());
            }
        }
        Ok(map)
    } else {
        Err(())
    }
}
