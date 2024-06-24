[![Review Assignment Due Date](https://classroom.github.com/assets/deadline-readme-button-24ddc0f5d75046c5622901739e7c5dd533143b0c8e959d652212380cedb1ea36.svg)](https://classroom.github.com/a/TXciPqtn)
# Rustwebserver

## Descriere generală
Acest program implementează un web server asincron folosind biblioteca Tokio din Rust. Serverul este capabil să gestioneze cereri HTTP GET și POST și poate servi fișiere dintr-un director specificat sau rula scripturi CGI pentru cererile către /scripts/.

## Cum este implementat
Programul este implementat în Rust, folosind Tokio pentru programarea asincronă. Acesta constă dintr-o funcție principală main care inițializează serverul și mai multe funcții auxiliare pentru a gestiona cererile HTTP și a răspunde corespunzător.

## Structura Codului
### 1.Biblioteci importate:

rust
---
use tokio::net::{TcpListener, TcpStream};
use tokio::fs;
use tokio::process::Command;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Arc;
---
### 2.Funcția principală main:

Aceasta inițializează serverul, verifică argumentele de linie de comandă, și configurează TcpListener pentru a asculta conexiunile pe un anumit port.
Folosește un buclă infinită pentru a accepta conexiuni și creează un nou task pentru fiecare conexiune primită folosind tokio::spawn.
rust
---
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
---
### 3.Funcția handle_connection:

Aceasta gestionează fiecare conexiune, citind cererea HTTP și delegând răspunsul corespunzător în funcție de metoda HTTP (GET sau POST) și de calea solicitată.
rust
---
async fn handle_connection(mut stream: TcpStream, root: Arc<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = [0; 8192];
    let size = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let (request_line, headers, body) = parse_request(&request);
    let (method, path, _) = process_request_line(&request_line);

    let client_ip = stream.peer_addr()?.ip().to_string();

    match method {
        "GET" => {
            if path.starts_with("/scripts/") {
                handle_script(&mut stream, &root, &path, &headers, &client_ip, "GET", &body).await?;
            } else {
                handle_get(&mut stream, &root, &path, &client_ip).await?;
            }
        },
        "POST" => {
            if path.starts_with("/scripts/") {
                handle_script(&mut stream, &root, &path, &headers, &client_ip, "POST", &body).await?;
            } else {
                send_response(&mut stream, 405, "Method Not Allowed", "text/html; charset=utf-8", "<html>405 Method Not Allowed</html>").await?;
            }
        },
        _ => {
            send_response(&mut stream, 405, "Method Not Allowed", "text/html; charset=utf-8", "<html>405 Method Not Allowed</html>").await?;
        }
    }

    Ok(())
}
---
### 4.Funcțiile auxiliare:

parse_request și process_request_line: Analizează cererea HTTP pentru a obține linia de cerere, anteturile și corpul cererii.
handle_get: Gestionează cererile GET, verificând dacă calea solicită un script sau un fișier/dosar și răspunzând corespunzător.
handle_script: Rulează scripturi CGI, setând variabilele de mediu corespunzătoare și capturând output-ul.
send_response și send_binary_response: Trimit răspunsuri HTTP text sau binare către client.
handle_directory_listing: Generează și trimite listarea directorului solicitat.
get_content_type: Determină tipul de conținut al fișierului solicitat.
log_request: Loghează detalii despre cererea procesată.
rust
---
fn parse_request(request: &str) -> (String, HashMap<String, String>, String) { ... }

fn process_request_line(request_line: &str) -> (&str, &str, &str) { ... }

async fn handle_get(stream: &mut TcpStream, root: &str, path: &str, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> { ... }

async fn send_binary_response(stream: &mut TcpStream, status_code: u32, status: &str, content_type: &str, content: &[u8]) -> Result<(), Box<dyn std::error::Error>> { ... }

async fn handle_directory_listing(stream: &mut TcpStream, full_path: &Path, display_path: &str, client_ip: &str) -> Result<(), Box<dyn std::error::Error>> { ... }

async fn handle_script(stream: &mut TcpStream, root: &str, path: &str, headers: &HashMap<String, String>, client_ip: &str, method: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> { ... }

async fn send_script_response(stream: &mut TcpStream, status_code: u32, status: &str, script_headers: &HashMap<String, String>, body: &str) -> Result<(), Box<dyn std::error::Error>> { ... }

fn get_content_type(path: &Path) -> String { ... }

async fn send_response(stream: &mut TcpStream, status_code: u32, status: &str, content_type: &str, message: &str) -> Result<(), Box<dyn std::error::Error>> { ... }

fn log_request(method: &str, client_ip: &str, path: &str, status_code: u32, status_text: &str) { ... }
---
## Cum funcționează
### 1.Inițializare:

Serverul este pornit cu cargo run <PORT> <ROOT_FOLDER>, unde <PORT> este portul pe care serverul va asculta conexiunile, iar <ROOT_FOLDER> este directorul rădăcină din care se vor servi fișierele.
### 2.Acceptarea conexiunilor:

Serverul ascultă pe adresa 0.0.0.0:<PORT> și acceptă conexiuni noi în buclă.
Pentru fiecare conexiune, se pornește un nou task pentru a gestiona cererea respectivă.
### 3.Procesarea cererilor:

Cererea HTTP este citită și analizată pentru a extrage linia de cerere, anteturile și corpul.
În funcție de metoda HTTP (GET sau POST) și de calea solicitată, serverul răspunde corespunzător:
GET: Dacă calea începe cu /scripts/, se rulează scriptul CGI; altfel, se servește fișierul sau se listează conținutul directorului.
POST: Dacă calea începe cu /scripts/, se rulează scriptul CGI; altfel, se răspunde cu "405 Method Not Allowed".
### 4.Rularea scripturilor CGI:

Scripturile sunt rulate folosind tokio::process::Command, cu variabilele de mediu setate pe baza anteturilor cererii și a altor informații relevante.
Rezultatul scriptului este capturat și trimis înapoi clientului.
### 5.Servirea fișierelor și directoarelor:

Fișierele sunt citite asincron și trimise înapoi clientului cu tipul de conținut corespunzător.
Directoarele sunt listate și conținutul generat este trimis ca răspuns HTML.
### 6.Logare și trimitere răspunsuri:

Fiecare cerere este logată cu detalii despre metodă, IP-ul clientului, calea solicitată și codul de status al răspunsului.
Răspunsurile sunt formate și trimise clientului, fie ca text HTML, fie ca conținut binar, în funcție de cerere și de resursele solicitate.
## Concluzie
Acest web server asincron în Rust folosește Tokio pentru a gestiona eficient cererile HTTP și a servi fișiere sau a rula scripturi CGI. Documentația de mai sus oferă o privire de ansamblu asupra implementării și funcționării sale, explicând principalele componente și funcționalități ale codului.