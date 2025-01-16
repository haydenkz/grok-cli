use std::env;
use std::error::Error;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::fs;
use std::collections::HashMap;
use reqwest;

pub struct Config {
    pub endpoint: String,
    pub key: String,
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
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // parse cli args into a string
    let args: Vec<String> = env::args().collect();
    // remove arg 0
    let args: Vec<String> = args[1..].to_vec();
    // join the args into a string
    let prompt: String = args.join(" ");
    
    grok(prompt).await?;
    Ok(())
}

async fn grok(prompt: String) -> Result<(), Box<dyn Error>> {
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
    
    println!("{}", response);
    Ok(())
}