mod common;

use std::fs;

use common::*;
use larpity_server::Config;

/// A pretend `client/` and `firmware/` with the files a checkout has.
fn folders() -> (std::path::PathBuf, std::path::PathBuf) {
    let client = scratch_dir("client");
    fs::write(client.join("index.html"), "<p>viewer</p>").unwrap();
    fs::create_dir(client.join("dist")).unwrap();
    fs::write(client.join("dist/main.js"), "console.log(1)").unwrap();
    fs::create_dir(client.join("src")).unwrap();
    fs::write(client.join("src/main.ts"), "export {}").unwrap();
    for name in ["package.json", "tsconfig.json", "CLAUDE.md", "README.md", ".gitignore"] {
        fs::write(client.join(name), "secret-ish").unwrap();
    }
    let firmware = scratch_dir("firmware");
    fs::write(firmware.join("index.html"), "<p>phone</p>").unwrap();
    fs::write(firmware.join("CLAUDE.md"), "notes").unwrap();
    (client, firmware)
}

#[tokio::test]
async fn pages_are_served_with_no_cache() {
    let (client, firmware) = folders();
    let addr = start(Config {
        client_dir: client.clone(),
        firmware_dir: firmware.clone(),
        ..Config::default()
    })
    .await;

    for (path, body) in [
        ("/", "<p>viewer</p>"),
        ("/index.html", "<p>viewer</p>"),
        ("/dist/main.js", "console.log(1)"),
        ("/phone/", "<p>phone</p>"),
        ("/phone/index.html", "<p>phone</p>"),
    ] {
        let response = http_get(addr, path).await;
        assert_eq!(response.status, 200, "{path}");
        assert_eq!(response.header("cache-control"), Some("no-cache"), "{path}");
        assert!(response.header("etag").is_some(), "{path} has no ETag");
        assert_eq!(response.body, body.as_bytes(), "{path}");
    }
    let redirect = http_get(addr, "/phone").await;
    assert_eq!(redirect.status, 308);
    assert_eq!(redirect.header("location"), Some("/phone/"));

    fs::remove_dir_all(client).unwrap();
    fs::remove_dir_all(firmware).unwrap();
}

#[tokio::test]
async fn developer_files_are_not_served() {
    let (client, firmware) = folders();
    let addr = start(Config {
        client_dir: client.clone(),
        firmware_dir: firmware.clone(),
        ..Config::default()
    })
    .await;

    for path in [
        "/package.json",
        "/tsconfig.json",
        "/CLAUDE.md",
        "/README.md",
        "/.gitignore",
        "/src/main.ts",
        "/CLAUDE%2Emd",
        "/phone/CLAUDE.md",
    ] {
        let response = http_get(addr, path).await;
        assert_eq!(response.status, 404, "{path} was served");
    }
    // A file that really isn't there is a plain 404 too.
    assert_eq!(http_get(addr, "/missing.js").await.status, 404);

    fs::remove_dir_all(client).unwrap();
    fs::remove_dir_all(firmware).unwrap();
}

#[tokio::test]
async fn health_still_answers() {
    let addr = start(Config::default()).await;
    let response = http_get(addr, "/health").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok");
}
