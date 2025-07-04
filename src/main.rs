// 强制将 RouteFunction 收集到 inventory 中
pub mod handler;
use actix_web::{App, HttpServer};
use route_codegen::generate_configure;
use std::env;

generate_configure!();

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    unsafe {
        env::set_var("RUST_LOG", "actix_web=info");
    }
    env_logger::init();

    println!("Starting HTTP server at http://127.0.0.1:8080");

    HttpServer::new(|| App::new().configure(configure))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
