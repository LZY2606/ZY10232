use pollen_stage::{db::Db, seed_if_needed, web, FIXTURE_JSON};
use std::sync::Arc;

struct Args {
    listen: String,
    db_path: String,
}

fn parse_args() -> Args {
    let mut listen = "127.0.0.1:5572".to_string();
    let mut db_path = "pollen-stage.sqlite3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => {
                if let Some(v) = args.next() {
                    listen = v;
                }
            }
            "--db" => {
                if let Some(v) = args.next() {
                    db_path = v;
                }
            }
            "-h" | "--help" => {
                println!("孢粉演替台 (pollen-succession-stage)");
                println!(
                    "用法: pollen-stage [--listen 127.0.0.1:5572] [--db pollen-stage.sqlite3]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("未知参数: {other}（用 --help 查看用法）");
                std::process::exit(2);
            }
        }
    }
    Args { listen, db_path }
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let db = Arc::new(Db::open(&args.db_path).expect("打开/初始化 SQLite 失败"));
    let hash = seed_if_needed(&db).expect("固定柱样校验/导入失败");
    let fx = pollen_stage::models::load_fixture(FIXTURE_JSON).expect("fixture");
    println!(
        "孢粉演替台：站点 {} | 固定柱样指纹 {} | 数据库 {}",
        fx.site, hash, args.db_path
    );
    let state = web::AppState {
        db,
        fixture_json: Arc::new(FIXTURE_JSON.to_string()),
    };
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .unwrap_or_else(|e| panic!("监听 {} 失败: {e}", args.listen));
    println!("访问 http://{}/ 查看“孢粉演替台”", args.listen);
    axum::serve(listener, web::app(state))
        .await
        .expect("服务器异常退出");
}
