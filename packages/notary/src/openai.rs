
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;


#[derive(Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
}

#[derive(Serialize)]
pub struct OpenAIThreadRequest {
    pub messages: Vec<Message>,
}


pub async fn create_thread() -> Result<(), Box<dyn std::error::Error>> {
    // Set your OpenAI API key here
    let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let client = Client::new();
    let request = OpenAIThreadRequest {
        messages: vec![Message {
            role: "user".to_string(),
            content: "Who is the current president of USA?".to_string(),
        }]
    };


    let list_messages_req = OpenAIThreadRequest {
        messages: vec![Message {
            role: "user".to_string(),
            content: "Who is the current president of USA?".to_string(),
        }]
    };


    let response = client
        .post("https://api.openai.com/v1/threads")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("OpenAI-Beta","assistants=v2")
        .header("OpenAI-Organization","org-gWqyehNS3iFul9AgQ27AkoXg")
        .json(&request)
        .send()
        // .json::<OpenAIResponse>()
        .await?;

        
    println!("Create Response: {}", response.text().await?);


    let list_response = client
        .get("https://api.openai.com/v1/threads/thread_JApUi9tQ7ucc22fyrA0lEK5z/messages")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("OpenAI-Beta","assistants=v2")
        .header("OpenAI-Organization","org-gWqyehNS3iFul9AgQ27AkoXg")

        .send()
        // .json::<OpenAIResponse>()
        .await?;
    

    println!("List Response: {}", list_response.text().await?);
// https://api.openai.com/v1/threads/{thread_id}/messages


    // if let Some(choice) = response.choices.first() {
    //     println!("Response: {}", choice.text.trim());
    // } else {
    //     println!("No response from OpenAI");
    // }

    Ok(())
}