use std::collections::HashMap;
use std::{
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex},
};

use futures::executor::block_on;
use url::Url;
use wsmq_rs::{client, server};

mod protos {
    pub mod basic;
}

fn test_client() {
    tokio::spawn(async {
        // connect to server
        match wsmq_rs::client::connect_with_config(
            &Url::parse("ws://127.0.0.1:9999").unwrap(),
            Some(client::Config {
                bandwidth: 1024 * 1024 * 16,
            }),
        )
        .await
        {
            Ok(client) => {
                // generate message
                let mut msg = protos::basic::BasicMessage::new();
                msg.set_value1("client ping".to_string());
                msg.set_value2(0);

                // send only
                println!("[client] send_message(client ping) only : 0");
                client.send_message(&msg).unwrap();

                // send and reply
                println!("[client] send_message(client ping) and reply : 1");
                msg.set_value2(1);
                match client.send_message(&msg) {
                    Ok(res) => {
                        println!("[client] wait for reply message(server pong)");
                        match res.await {
                            Ok(res) => match res.to_message::<protos::basic::BasicMessage>() {
                                Ok(mut message) => {
                                    assert_eq!(message.get_value1(), "server pong");
                                    assert_eq!(message.value2, 2000);
                                    message.set_value1("client pong".to_string());
                                    message.set_value2(2);
                                    println!("[client] send_message(client pong) only : 2");
                                    match res.send_message(&message).await {
                                        Ok(res) => match res.await {
                                            Ok(res) => {
                                                if let Ok(message) =
                                                    res.to_message::<protos::basic::BasicMessage>()
                                                {
                                                    message.get_value1();
                                                }
                                            }
                                            Err(err) => println!(
                                                "[client] Failed to send_message(client pong) : {}",
                                                err.cause()
                                            ),
                                        },
                                        Err(err) => println!(
                                            "[client] Failed to Response(client pong) : {}",
                                            err.cause()
                                        ),
                                    }
                                }
                                Err(err) => {
                                    println!("[client] Failed to to_message : {}", err.cause())
                                }
                            },
                            Err(err) => println!("[client] Failed to Response : {}", err.cause()),
                        }
                    }
                    Err(err) => println!(
                        "[client] Failed to send_message(client ping) : {}",
                        err.cause()
                    ),
                }
            }
            Err(err) => println!(
                "[client] Failed to connect_with_config(client) : {}",
                err.cause()
            ),
        }
    });
}

#[tokio::test]
async fn test_server() {
    tokio::spawn(async {
        let client_map = Arc::new(Mutex::new(HashMap::new()));
        let client_map_on_started = client_map.clone();
        let client_map_on_message = client_map.clone();
        let client_map_on_connected = client_map.clone();
        if let Err(err) = wsmq_rs::server::run(
            &SocketAddr::from_str("0.0.0.0:9999").unwrap(),
            server::Config::new(1024 * 1024 * 16, move |addr, res| {
                println!("[server] On message : {}", addr);
                let mut message = res.to_message::<protos::basic::BasicMessage>().unwrap();
                let client_map = client_map_on_message.clone();
                if let Ok(mut map) = client_map.lock() {
                    if let Some(ctx) = map.get_mut(&addr) {
                        assert_eq!(message.value2, *ctx);
                        *ctx += 1;
                        println!("[server] send_message(server pong) : {}", ctx);
                        message.set_value1("server pong".to_string());
                        message.set_value2(*ctx * 1000);
                        match block_on(res.send_message(&message)) {
                            Ok(res) => match block_on(res) {
                                Ok(_res) => {
                                    //res.send_message(&message).await;
                                }
                                Err(err) => println!(
                                    "[server] Failed to Response(server pong) : {}",
                                    err.cause()
                                ),
                            },
                            Err(err) => println!(
                                "[server] Failed to send_message(server pong) : {}",
                                err.cause()
                            ),
                        }
                    }
                }
                ()
            })
            .on_started(Box::new(move || {
                if let Ok(_map) = client_map_on_started.lock() {}
                println!("[server] Started");
                test_client();
            }))
            .on_connect(Box::new(move |addr| {
                let client_map = client_map_on_connected.clone();
                client_map.lock().unwrap().insert(addr, 0);
                println!("[server] Connected : {}", addr);
            }))
            .on_disconnect(Box::new(move |addr| {
                println!("[server] Disconnected : {}", addr);
            }))
            .on_error(Box::new(move |err| {
                println!("[server] Failed to run_with_config(client) : {}", err);
            })),
        )
        .await
        {
            println!(
                "[server] Failed to run_with_config(client) : {}",
                err.cause()
            );
        }
    })
    .await
    .unwrap();
}
