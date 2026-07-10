use are_you_up_backend::{app, open_db};

#[tokio::main]
async fn main() {
    let addr = std::env::var("ARE_YOU_UP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let db_path = std::env::var("ARE_YOU_UP_DB").unwrap_or_else(|_| "./are-you-up.db".into());
    let conn = open_db(&db_path);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind ARE_YOU_UP_ADDR; the address must be free and well-formed");
    println!("listening on {addr}, database at {db_path}");
    axum::serve(listener, app(conn)).await.expect("serve fails only on unrecoverable accept errors");
}
