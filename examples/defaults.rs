use std::{env, process};

use launchdarkly_rust_sdk_alt::DefaultClient;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Please pass a token as the first argument");
        process::exit(1);
    }
    let token = &args[1];

    let mut client = DefaultClient::with_token(token.into()).expect("invalid token");
    client.start().await.expect("failed to start");
    dbg!(client.export());
}
