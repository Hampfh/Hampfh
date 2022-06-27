use dotenv::dotenv;

use crate::repo_updater::is_live;

pub fn create_issue_comment(issue_number: i32, msg: &str) {
    dotenv().ok();

    let secret = std::env::var("GITHUB_POST_SECRET").unwrap();
    let user = std::env::var("GITHUB_USER").unwrap_or("hampfh".to_string());
    let repo = std::env::var("GITHUB_REPO").unwrap_or("temp".to_string());
    if !is_live() {
        println!("[OFFLINE] Issue comment: {}", msg);
        return;
    }

    // use reqwest to send a post request to https://api.github.com
    let client = reqwest::blocking::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments",
        user, repo, issue_number
    );
    let req = client
        .post(&url)
        .header("User-Agent", user)
        .header("Accept", "application/vnd.github.v3+json")
        .header("Authorization", format!("token {}", secret))
        .body(format!(
            "{{\"body\": \"[THIS MESSAGE IS AUTOMATIC]<br> {}\"}}",
            msg
        ));

    // Send request
    req.send().unwrap();
}
