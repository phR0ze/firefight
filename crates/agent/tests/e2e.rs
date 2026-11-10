//! Server + agent in one process: pair, assign, start, and verify the agent
//! executes the timer's command when it finishes.

use std::path::PathBuf;
use std::time::Duration;

use firefight_agent::config::AgentConfig;
use firefight_core::model::{ClientStatus, ClientView, TimerDef};
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

struct Env {
    dir: PathBuf,
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Env {
    async fn start() -> Self {
        Self::start_with(|_| (firefight_server::Options::default(), None)).await
    }

    /// `setup` gets the test directory and returns the server options and the agent's enroll key.
    async fn start_with(setup: impl FnOnce(&PathBuf) -> (firefight_server::Options, Option<String>)) -> Self {
        let dir = std::env::temp_dir().join(format!("firefight-e2e-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (opts, enroll_key) = setup(&dir);
        let state = firefight_server::init_with(&dir.join("server"), &opts).unwrap();
        let token = state.admin_token.to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(firefight_server::serve(listener, state));

        let cfg = AgentConfig {
            server: addr.to_string(),
            name: "test-agent".into(),
            ui_port: 0,
            client_id: None,
            token: None,
            enroll_key,
        };
        let ui = TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(firefight_agent::run(cfg, Some(dir.join("agent.toml")), ui));

        Env { base: format!("http://{addr}/api/admin"), dir, token, http: reqwest::Client::new() }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> T {
        let res = self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token).send().await.unwrap();
        assert!(res.status().is_success(), "GET {path}: {}", res.status());
        res.json().await.unwrap()
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: serde_json::Value) -> serde_json::Value {
        let res = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    }

    async fn wait_for<T>(&self, what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
        for _ in 0..100 {
            if let Some(v) = f().await {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}");
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn paired_agent_runs_command_when_timer_finishes() {
    let env = Env::start().await;
    let marker = env.dir.join("fired");

    // Admin auth is enforced.
    let res = env.http.get(format!("{}/timers", env.base)).send().await.unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The agent shows up as pending, then gets approved.
    let client = env
        .wait_for("pending client", async || {
            let clients: Vec<ClientView> = env.get("/clients").await;
            clients.into_iter().find(|c| c.online)
        })
        .await;
    assert_eq!(client.status, ClientStatus::Pending);
    env.send(reqwest::Method::POST, &format!("/clients/{}/approve", client.id), json!({})).await;

    // Credentials were persisted for the agent.
    env.wait_for("agent token saved", async || {
        let cfg = AgentConfig::load(&env.dir.join("agent.toml")).unwrap()?;
        cfg.token.map(|_| ())
    })
    .await;

    let timer: TimerDef = serde_json::from_value(
        env.send(
            reqwest::Method::POST,
            "/timers",
            json!({
                "name": "e2e",
                "kind": { "type": "countdown", "duration_secs": 1 },
                "actions": [{
                    "id": Uuid::new_v4(),
                    "kind": { "type": "command", "cmd": format!("touch '{}'", marker.display()) },
                    "mode": "parallel"
                }]
            }),
        )
        .await,
    )
    .unwrap();
    env.send(reqwest::Method::PUT, &format!("/clients/{}/assignments", client.id), json!({ "timers": [timer.id] }))
        .await;
    // Let the assignment reach the agent before starting.
    tokio::time::sleep(Duration::from_millis(200)).await;
    env.send(reqwest::Method::POST, &format!("/timers/{}/start", timer.id), json!({})).await;

    env.wait_for("command to run", async || marker.exists().then_some(())).await;

    let result = env
        .wait_for("action result reported", async || {
            let clients: Vec<ClientView> = env.get("/clients").await;
            clients.into_iter().find(|c| c.id == client.id)?.last_result
        })
        .await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.timer_id, Some(timer.id));
}

#[tokio::test]
async fn unassigned_agent_does_not_run_actions() {
    let env = Env::start().await;
    let marker = env.dir.join("fired");
    let client = env
        .wait_for("pending client", async || {
            let clients: Vec<ClientView> = env.get("/clients").await;
            clients.into_iter().find(|c| c.online)
        })
        .await;
    env.send(reqwest::Method::POST, &format!("/clients/{}/approve", client.id), json!({})).await;
    let timer = env
        .send(
            reqwest::Method::POST,
            "/timers",
            json!({
                "name": "other",
                "kind": { "type": "countdown", "duration_secs": 1 },
                "actions": [{
                    "id": Uuid::new_v4(),
                    "kind": { "type": "command", "cmd": format!("touch '{}'", marker.display()) }
                }]
            }),
        )
        .await;
    env.send(reqwest::Method::POST, &format!("/timers/{}/start", timer["id"].as_str().unwrap()), json!({})).await;
    tokio::time::sleep(Duration::from_millis(1800)).await;
    assert!(!marker.exists());
}

#[tokio::test]
async fn preconfigured_server_and_agent_need_no_manual_steps() {
    let env = Env::start_with(|dir| {
        std::fs::write(dir.join("admin_token"), "admin-from-file\n").unwrap();
        std::fs::write(dir.join("enroll_key"), "enroll-from-file\n").unwrap();
        let marker = dir.join("fired");
        let config = json!({
            "timers": [{
                "key": "e2e",
                "name": "Declared",
                "kind": { "type": "countdown", "duration_secs": 1 },
                "actions": [{ "kind": { "type": "command", "cmd": format!("touch '{}'", marker.display()) } }]
            }]
        });
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
        let opts = firefight_server::Options {
            admin_token_file: Some(dir.join("admin_token")),
            enroll_key_file: Some(dir.join("enroll_key")),
            config: Some(dir.join("config.json")),
        };
        (opts, Some("enroll-from-file".into()))
    })
    .await;
    assert_eq!(env.token, "admin-from-file", "admin token comes from the file, trimmed");

    let timer_id = firefight_server::config::timer_id("e2e");
    let timers: Vec<TimerDef> = env.get("/timers").await;
    assert!(timers.iter().any(|t| t.id == timer_id && t.name == "Declared"), "declared timer created");

    let client = env
        .wait_for("enrolled client", async || {
            let clients: Vec<ClientView> = env.get("/clients").await;
            clients.into_iter().find(|c| c.online && c.status == ClientStatus::Approved)
        })
        .await;
    assert!(client.assigned_timers.is_empty(), "the config assigns nothing; that's the deck's job");

    // Assign it as the deck would, let the assignment reach the agent, then run the timer.
    env.send(reqwest::Method::PUT, &format!("/clients/{}/assignments", client.id), json!({ "timers": [timer_id] }))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    env.send(reqwest::Method::POST, &format!("/timers/{timer_id}/start"), json!({})).await;
    env.wait_for("declared command to run", async || env.dir.join("fired").exists().then_some(())).await;
}
