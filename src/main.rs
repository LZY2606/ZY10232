//! 孢粉演替台本地服务入口。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use pollen_stage::{db, fixture, server};

struct Args {
    listen: SocketAddr,
    db: String,
}

fn parse_args() -> Args {
    let mut listen: SocketAddr = "127.0.0.1:5572".parse().unwrap();
    let mut db = "pollen-stage.sqlite3".to_string();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => {
                let v = it.next().expect("--listen 需要地址 host:port");
                listen = v.parse().expect("无法解析监听地址");
            }
            "--db" => db = it.next().expect("--db 需要路径"),
            "--help" | "-h" => {
                println!("用法: pollen-stage [--listen 127.0.0.1:5572] [--db PATH]");
                std::process::exit(0);
            }
            other => panic!("未知参数 {other}（用 --help 查看用法）"),
        }
    }
    Args { listen, db }
}

#[tokio::main]
async fn main() {
    let args = parse_args();

    let mut conn = db::open(&args.db).expect("打开 SQLite 失败");
    db::init_schema(&conn).expect("初始化 schema 失败");
    let empty = db::is_empty(&conn).expect("检查数据库失败");
    if empty {
        fixture::seed(&mut conn).expect("写入固定 fixture 失败");
        server::seed_demo_runs(&mut conn).expect("写入演示运行失败");
        println!("已初始化数据库并写入固定 fixture：{}", args.db);
    }

    let state = server::AppState {
        db_path: args.db.clone(),
        conn: Arc::new(Mutex::new(conn)),
    };
    let app = server::router(state);

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .expect("绑定监听地址失败");
    println!("孢粉演替台已启动：http://{}", args.listen);
    println!("数据文件：{}", args.db);
    axum::serve(listener, app).await.expect("服务失败");
}
