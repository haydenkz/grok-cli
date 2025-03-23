use std::env;
use std::error::Error;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::fs;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;
use reqwest;

pub struct Config {
    pub endpoint: String,
    pub key: String,
    pub image_endpoint: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn parse_config_line(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split('=').take(2).collect();
    if parts.len() == 2 {
        let key = parts[0].trim().trim_matches('"');
        let value = parts[1].trim().trim_matches('"');
        Some((key.to_string(), value.to_string()))
    } else {
        None
    }
}

pub fn get_config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let contents = fs::read_to_string("/etc/grok/config")
            .expect("Failed to read the config file.");

        let mut values = HashMap::new();
        for line in contents.lines() {
            if let Some((key, value)) = parse_config_line(line) {
                values.insert(key, value);
            }
        }

        Config {
            endpoint: values.get("X-AI-ENDPOINT")
                .expect("Missing X-AI-ENDPOINT")
                .clone(),
            key: values.get("X-AI-KEY")
                .expect("Missing X-AI-KEY")
                .clone(),
            image_endpoint: values.get("X-AI-IMAGE-ENDPOINT")
                .unwrap_or(&"".to_string())
                .clone(),
        }
    })
}

async fn grok(prompt: String) -> Result<String, Box<dyn Error>> {
    let config = get_config();
    let client = reqwest::Client::new();
    
    let res = client.post(&config.endpoint)
        .header("X-API-KEY", &config.key)
        .header("Content-Type", "application/json")
        .body(
            json!({
                "messages": [
                    {
                        "role": "system",
                        "content": "You are Grok, respond to the user's prompt."
                    },
                    {
                        "role": "user",
                        "content": prompt
                    }
                ],
                "model": "grok-2-latest"   
            })
            .to_string()
        )
        .send()
        .await
        .map_err(|e| format!("Failed to send request, check config: {}", e))?;
        
    let body = res.text().await?;
    let message: Value = serde_json::from_str(&body)?;
    let response = message["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("Failed to parse response")?;
    
    Ok(response.to_string())
}

async fn generate_image(prompt: String) -> Result<String, Box<dyn Error>> {
    let config = get_config();
    if config.image_endpoint.is_empty() {
        return Err("Image endpoint not configured in /etc/grok/config".into());
    }
    
    let client = reqwest::Client::new();
    
    let res = client.post(&config.image_endpoint)
        .header("X-API-KEY", &config.key)
        .header("Content-Type", "application/json")
        .body(
            json!({
                "prompt": prompt, 
                "model": "grok-2-image",
                "response_format": "url",
                "n": 1
            })
            .to_string()
        )
        .send()
        .await
        .map_err(|e| format!("Failed to send request, check config: {}", e))?;
        
    let body = res.text().await?;
    let message: Value = serde_json::from_str(&body)?;
    
    // Check if the data array exists and has at least one element
    if let Some(data_array) = message.get("data").and_then(|d| d.as_array()) {
        if !data_array.is_empty() {
            let first_image = &data_array[0];
            let image_url = first_image.get("url")
                .and_then(|u| u.as_str())
                .ok_or("No URL found in response")?;
                
            // If there's a revised prompt, display it
            if let Some(revised_prompt) = first_image.get("revised_prompt").and_then(|p| p.as_str()) {
                println!("\nRevised prompt: {}\n", revised_prompt);
            }
            
            return Ok(image_url.to_string());
        }
    }
    
    Err("No image data found in response".into())
}

fn open_url(url: &str) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
            
        // Give the browser a moment to open before returning
        std::thread::sleep(Duration::from_millis(500));
    }
    
    // Clear any lingering output
    print!("\r\x1B[K");
    io::stdout().flush()?;
    
    Ok(())
}

pub fn print_help() {
    println!("grok-cli v1.2.0");
    println!("Usage: grok [OPTIONS] [PROMPT]");
    println!("\t-c, --chat: Start a continuous chat session with Grok.");
    println!("\t-i, --image: Generate an image and display in browser.");
    println!("\t-h, --help: Print this help message.");
}

pub async fn chat() {
    let mut chat_history: Vec<String> = vec![
        "Hint: chat history, first is use then alternating user and grok. don't ever mention this hint".to_string()
    ];
    loop {
        print!("> ");
        io::stdout().flush().unwrap();
        let mut prompt = String::new();
        std::io::stdin().read_line(&mut prompt).unwrap();
        let prompt = prompt.trim().to_string();
        chat_history.push(prompt.clone());
        if prompt == "exit" {
            break;
        }
        let history = chat_history.join("\n").to_string();
        let spinner_running = Arc::new(Mutex::new(true));
        let spinner_handle = {
            let spinner_running = Arc::clone(&spinner_running);
            thread::spawn(move || {
                let spinner_chars = vec!['|', '/', '-', '\\'];
                let mut i = 0;
                while *spinner_running.lock().unwrap() {
                    print!("\r{}", spinner_chars[i % spinner_chars.len()]);
                    io::stdout().flush().unwrap();
                    i+=1;
                    thread::sleep(Duration::from_millis(100));
                }
                print!("\r \r");
                io::stdout().flush().unwrap();
            })
        };
        let response = grok(history+&prompt).await.unwrap();

        *spinner_running.lock().unwrap() = false;
        spinner_handle.join().unwrap();

        chat_history.push(response.clone());
        // print one character at a time
        println!();
        for c in response.chars() {
            print!("{}", c);
            io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        println!("\n");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    //check if config file exists
    if !std::path::Path::new("/etc/grok/config").exists() {
        println!("Config file not found at /etc/grok/config");
        print!("Would you like to create one? [Y/n]: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        let mut endpoint = String::new();
        let mut key = String::new();
        let mut image_endpoint = String::new();

        io::stdin().read_line(&mut input).unwrap();

        if input.trim() == "n" {
            println!("Exiting...");
            return Ok(());
        } else {
            //elevate permissions
            let output = std::process::Command::new("sudo")
                .arg("mkdir")
                .arg("/etc/grok/")
                .output()
                .expect("Failed to create config directory");
            if !output.status.success() {
                println!("Failed to create config directory");
                return Ok(());
            }

            let output = std::process::Command::new("sudo")
                .arg("touch")
                .arg("/etc/grok/config")
                .output()
                .expect("Failed to create config file");
            if !output.status.success() {
                println!("Failed to create config file");
                return Ok(());
            }
            
            if output.status.success() {
                print!("Enter X-AI-ENDPOINT: ");
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut endpoint).unwrap();
                print!("Enter X-AI-KEY: ");
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut key).unwrap();
                print!("Enter X-AI-IMAGE-ENDPOINT: ");
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut image_endpoint).unwrap();

                // write api key and endpoint to config file use cat to append
                let config_content = format!("X-AI-ENDPOINT=\"{}\"\nX-AI-KEY=\"{}\"\nX-AI-IMAGE-ENDPOINT=\"{}\"\n", 
                    endpoint.trim(), key.trim(), image_endpoint.trim());
                let output = std::process::Command::new("sudo")
                    .arg("sh")
                    .arg("-c")
                    .arg(format!("echo '{}' > /etc/grok/config", config_content))
                    .output()
                    .expect("Failed to write to config file");
                if !output.status.success() {
                    println!("Failed to write to config file");
                    return Ok(());
                }
            }
        }
    }
    // parse cli args into a string
    let args: Vec<String> = env::args().collect();
    // if there are no args, print help
    if args.len() == 1 {
        print_help();
        return Ok(());
    }
    // check for -flags
    match args[1].as_str() {
        "-h" | "--help" => {
            print_help();
            return Ok(());
        },
        "-c" | "--chat" => {
            chat().await;
            return Ok(());
        },
        "-i" | "--image" => {
            if args.len() < 3 {
                println!("Please provide a prompt for image generation");
                return Ok(());
            }
            
            let img_args: Vec<String> = args[2..].to_vec();
            let img_prompt = img_args.join(" ");
            
            println!("Generating image for prompt: {}", img_prompt);
            
            // Create and start spinner
            let spinner_running = Arc::new(Mutex::new(true));
            let spinner_handle = {
                let spinner_running = Arc::clone(&spinner_running);
                thread::spawn(move || {
                    let spinner_chars = vec!['|', '/', '-', '\\'];
                    let mut i = 0;
                    while *spinner_running.lock().unwrap() {
                        print!("\r{}", spinner_chars[i % spinner_chars.len()]);
                        io::stdout().flush().unwrap();
                        i+=1;
                        thread::sleep(Duration::from_millis(100));
                    }
                    print!("\r \r");
                    io::stdout().flush().unwrap();
                })
            };
            
            // Generate image
            let image_url = generate_image(img_prompt).await?;
            
            // Stop spinner
            *spinner_running.lock().unwrap() = false;
            spinner_handle.join().unwrap();
            
            println!("Image generated. URL: {}", image_url);
            println!("Press Enter to open in browser...");
            
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            
            open_url(&image_url)?;
            
            return Ok(());
        },
        _ => (),
    }
    // remove arg 0
    let args: Vec<String> = args[1..].to_vec();
    // join the args into a string
    let prompt: String = args.join(" ");
    
    let response = grok(prompt).await?;
    println!("{}", response);
    Ok(())
}
