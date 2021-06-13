use futures::executor::block_on;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use wsmq_rs::{client, server};

mod protos {
    pub mod test;
}

macro_rules! define_test_future {
    ($test_code:expr) => {{
        let (svc, inst) = service_rs::service::Service::new();
        let svc = Arc::new(Mutex::new(svc));
        let inst1 = inst.clone();
        let test_server = async move { $test_code(svc, inst1).await };
        tokio::runtime::Handle::current().spawn_blocking(move || {
            match inst.do_events(tokio::spawn(test_server)) {
                Ok(event) => match event {
                    service_rs::service::Event::ServiceStatus(status) => match status {
                        service_rs::service::ServiceStatus::Stopped() => {
                            println!("Service stopped");
                            return;
                        }
                        service_rs::service::ServiceStatus::Paused(_) => {}
                        service_rs::service::ServiceStatus::Running() => {}
                    },
                    service_rs::service::Event::Future(result) => match result {
                        Ok(result) => {
                            println!("Result : {:?}", result);
                        }
                        Err(err) => {
                            println!("Elapsed : {}", err);
                        }
                    },
                },
                Err(err) => {
                    println!("Failed to do_events : {}", err);
                }
            }
        })
    }};
}

#[tokio::test]
async fn basic_test() {
    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client() {
            let client = wsmq_rs::client::connect("ws://127.0.0.1:65000")
                .await
                .expect("[client] Failed to client::connect");
            let mut message = protos::test::TestMessage::new();
            message.set_caption("client ping".to_string());
            message.set_seq(1);
            message.set_need_to_rely(true);
            println!("[client] send_message({:?})", message);
            let res = client
                .send_message(&message)
                .unwrap()
                .await
                .expect("Failed to send_message");
            let message = res
                .to_message::<protos::test::TestMessage>()
                .expect("[client] Failed to to_message");
            println!("[client] Message received ({:?})", message);
            assert_eq!(message.get_caption(), "server pong");
            assert_eq!(message.get_seq(), 1);
            println!("[client] Done");
        }
        wsmq_rs::server::run_with_config(
            "0.0.0.0:65000",
            move |addr, res, _| {
                let mut message = res
                    .to_message::<protos::test::TestMessage>()
                    .expect("[server] Failed to to_message");
                println!("[server] message received({:?}) : {} ", message, addr);
                assert_eq!(message.get_caption(), "client ping");
                assert_eq!(message.get_seq(), 1);
                message.set_caption("server pong".to_string());
                block_on(res.reply_message(&message)).expect("[server] Failed to reply_message");
                println!("[server] send_message({:?}) : {} ", message, addr);
                println!("[server] Done");
            },
            server::Config::<()>::new(1024 * 1024 * 16).on_started(Box::new(move || {
                let svc = svc.clone();
                tokio::spawn(async move {
                    test_client().await;
                    svc.lock().unwrap().stop().unwrap();
                });
            })),
        )
        .await
        .unwrap();
    });
    match tokio::time::timeout(Duration::from_secs(10), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn basic_test_err() {
    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client() {
            // connect to server
            match wsmq_rs::client::connect("ws://127.0.0.1:65001").await {
                Ok(client) => {
                    // generate message
                    let mut msg = protos::test::TestMessage::new();
                    msg.set_caption("client ping".to_string());
                    // send
                    msg.set_seq(1);
                    msg.set_need_to_rely(true);
                    println!("[client] send_message({:?})", msg);
                    match client.send_message(&msg) {
                        Ok(res) => {
                            println!("[client] wait for reply message({:?})", msg);
                            match res.await {
                                Ok(res) => match res.to_message::<protos::test::TestMessage>() {
                                    Ok(message) => {
                                        println!("[client] message received ({:?})", message);
                                    }
                                    Err(_) => {}
                                },
                                Err(_) => {}
                            }
                        }
                        Err(err) => println!(
                            "[client] Failed to send_message({:?}) : {}",
                            msg,
                            err.cause()
                        ),
                    }
                }
                Err(err) => println!("[client] Failed to connect : {}", err.cause()),
            }
        }

        if let Err(err) = wsmq_rs::server::run_with_config(
            "0.0.0.0:65001",
            move |addr, res, _| match res.to_message::<protos::test::TestMessage>() {
                Ok(mut message) => {
                    println!("[server] On message : {}, {:?}", addr, message);
                    message.set_caption("server pong".to_string());
                    match block_on(res.reply_message(&message)) {
                        Ok(_) => {
                            println!("[server] Done");
                        }
                        Err(err) => println!(
                            "[server] Failed to reply_message({:?}) : {}",
                            message,
                            err.cause()
                        ),
                    }
                }
                Err(err) => println!("[server] Failed to to_message : {}", err.cause()),
            },
            server::Config::new(1024 * 1024 * 16)
                .on_started(Box::new(move || {
                    println!("[server] Started");
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        test_client().await;
                        svc.lock().unwrap().stop().unwrap();
                    });
                }))
                .on_connect(Box::new(move |addr| {
                    println!("[server] Connected : {}", addr);
                }))
                .on_disconnect(Box::new(move |addr, _| {
                    println!("[server] Disconnected : {}", addr);
                }))
                .on_error(Box::new(move |err, _| {
                    println!("[server] Error : {}", err);
                })),
        )
        .await
        {
            println!("[server] Failed to run : {}", err.cause());
        }
    });

    match tokio::time::timeout(Duration::from_secs(10), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn send_large_message_test() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let f = define_test_future!(
        |svc: Arc<Mutex<service_rs::service::Service>>,
         inst: Arc<service_rs::service::ServiceInstance>| async move {
            async fn test_client(
                test_data: Vec<u8>,
                svc: Arc<Mutex<service_rs::service::Service>>,
                inst: Arc<service_rs::service::ServiceInstance>,
            ) {
                let client = wsmq_rs::client::connect_with_config(
                    "ws://127.0.0.1:65000",
                    client::Config::new(1024 * 1024 * 4).on_error(Box::new(|err| {
                        println!("[client] on_error : {}", err);
                    })),
                )
                .await
                .expect("[client] Failed to client::connect");
                let mut message = protos::test::TestMessage::new();
                message.set_caption("client ping".to_string());
                message.set_seq(1);
                message.set_payload(test_data);
                message.set_need_to_rely(false);
                println!(
                    "[client] send_message({}, {})",
                    message.caption,
                    message.payload.len()
                );
                match client.send_message(&message) {
                    Ok(res) => match res.await {
                        Ok(_) => {}
                        Err(err) => {
                            println!("[client] Failed to res.await({})", err);
                            svc.lock().unwrap().stop().unwrap();
                            return;
                        }
                    },
                    Err(err) => {
                        println!("[client] Failed to send_message({})", err);
                        svc.lock().unwrap().stop().unwrap();
                        return;
                    }
                };
                loop {
                    if inst.is_running() {
                        tokio::task::yield_now().await;
                    } else {
                        break;
                    }
                }
                println!("[client] Done");
            }

            println!("[common] Generate test_data");
            let test_data = Arc::new(
                (0..1024 * 1024 * 128)
                    .map(|f| (f % 255) as u8)
                    .collect::<Vec<u8>>(),
            );
            let test_data2 = test_data.clone();
            println!("[common] Generate test_data done.");

            let svc1 = svc.clone();
            wsmq_rs::server::run_with_config(
                "0.0.0.0:65000",
                move |addr, res, _| {
                    let message = res
                        .to_message::<protos::test::TestMessage>()
                        .expect("[server] Failed to to_message");
                    println!(
                        "[server] message received({}, {}) : {} ",
                        message.caption,
                        message.payload.len(),
                        addr
                    );
                    assert_eq!(message.get_caption(), "client ping");
                    assert_eq!(message.get_seq(), 1);
                    assert_eq!(*test_data, message.payload);
                    println!("[server] Done");
                    svc.lock().unwrap().stop().unwrap();
                },
                server::Config::<()>::new(1024 * 1024 * 4)
                    .on_started(Box::new(move || {
                        let svc = svc1.clone();
                        let inst = inst.clone();
                        let test_data = test_data2.to_vec();
                        rt.spawn(async move {
                            test_client(test_data, svc, inst).await;
                        });
                    }))
                    .on_progress(Box::new(|ctx, _| {
                        println!("[server] progress({:?})", ctx);
                    }))
                    .on_disconnect(Box::new(|addr, ctx| {
                        println!("[server] disconnected({:?}, {})", ctx, addr);
                    }))
                    .on_error(Box::new(|err, ctx| {
                        if let Some(ctx) = ctx {
                            println!("error({:?}) : {}", ctx, err);
                        } else {
                            println!("error : {}", err);
                        }
                    })),
            )
            .await
            .unwrap();
        }
    );
    match tokio::time::timeout(Duration::from_secs(60), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn send_large_message_ping_pong_test() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client(test_data: Vec<u8>) {
            let client = wsmq_rs::client::connect_with_config(
                "ws://127.0.0.1:65000",
                client::Config::new(1024 * 1024 * 4)
                    .on_progress(Box::new(|ctx| {
                        println!(
                            "[client] {:?} : {}/{}",
                            ctx.method, ctx.current, ctx.total_length
                        );
                    }))
                    .on_error(Box::new(|err| {
                        println!("[client] on_error : {}", err);
                    })),
            )
            .await
            .expect("[client] Failed to client::connect");
            let mut message = protos::test::TestMessage::new();
            message.set_caption("client ping".to_string());
            message.set_seq(1);
            message.set_payload(test_data);
            message.set_need_to_rely(true);
            println!(
                "[client] send_message({}, {})",
                message.caption,
                message.payload.len()
            );
            let res = client
                .send_message(&message)
                .unwrap()
                .await
                .expect("Failed to send_message");
            let message_res = res
                .to_message::<protos::test::TestMessage>()
                .expect("[client] Failed to to_message");
            println!(
                "[client] Message received ({}, {})",
                message_res.caption,
                message_res.payload.len()
            );
            assert_eq!(message_res.get_caption(), "server pong");
            assert_eq!(message_res.get_seq(), 1);
            assert_eq!(message.payload, message_res.payload);
            println!("[client] Done");
        }

        let test_data = Arc::new(
            (0..1024 * 1024 * 128)
                .map(|f| (f % 255) as u8)
                .collect::<Vec<u8>>(),
        );
        let test_data2 = test_data.clone();
        wsmq_rs::server::run_with_config(
            "0.0.0.0:65000",
            move |addr, res, _| {
                let mut message = res
                    .to_message::<protos::test::TestMessage>()
                    .expect("[server] Failed to to_message");
                println!(
                    "[server] message received({}, {}) : {} ",
                    message.caption,
                    message.payload.len(),
                    addr
                );
                assert_eq!(message.get_caption(), "client ping");
                assert_eq!(message.get_seq(), 1);
                assert_eq!(*test_data, message.payload);
                message.set_caption("server pong".to_string());
                block_on(res.reply_message(&message)).expect("[server] Failed to reply_message");
                println!(
                    "[server] send_message({}, {}) : {} ",
                    message.caption,
                    message.payload.len(),
                    addr
                );
                println!("[server] Done");
            },
            server::Config::<()>::new(1024 * 1024 * 16)
                .on_started(Box::new(move || {
                    let svc = svc.clone();
                    let test_data = test_data2.to_vec();
                    rt.spawn(async move {
                        test_client(test_data).await;
                        svc.lock().unwrap().stop().unwrap();
                    });
                }))
                .on_progress(Box::new(|ctx, _| {
                    println!("[server] progress({:?})", ctx);
                }))
                .on_error(Box::new(|err, ctx| {
                    if let Some(ctx) = ctx {
                        println!("[server] error({:?}) : {}", ctx, err);
                    } else {
                        println!("[server] error : {}", err);
                    }
                })),
        )
        .await
        .unwrap();
    });
    match tokio::time::timeout(Duration::from_secs(60), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn context_test() {
    struct TestContext {
        pub number: u16,
    }

    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client() {
            let client = wsmq_rs::client::connect("ws://127.0.0.1:65000")
                .await
                .expect("[client] Failed to client::connect");
            let mut message = protos::test::TestMessage::new();
            message.set_caption("client ping".to_string());
            message.set_seq(1);
            message.set_need_to_rely(true);
            println!("[client] send_message({:?})", message);
            let res = client
                .send_message(&message)
                .unwrap()
                .await
                .expect("Failed to send_message");
            let message = res
                .to_message::<protos::test::TestMessage>()
                .expect("[client] Failed to to_message");
            println!("[client] Message received ({:?})", message);
            assert_eq!(message.get_caption(), "server pong");
            assert_eq!(message.get_seq(), 1);
            println!("[client] Done");
        }
        wsmq_rs::server::run_with_config(
            "0.0.0.0:65000",
            move |addr, res, ctx| {
                let x = match addr {
                    SocketAddr::V4(ip) => ip.port() - 1,
                    SocketAddr::V6(ip) => ip.port() - 1,
                };
                let y = ctx.number;
                println!("on_message({}) : {} = {}", addr, x, y);
                assert_eq!(x, y);
                ctx.number = 10;
                println!("change ctx.number to {}", 10);
                let mut message = res
                    .to_message::<protos::test::TestMessage>()
                    .expect("[server] Failed to to_message");
                println!("[server] message received({:?}) : {} ", message, addr);
                assert_eq!(message.get_caption(), "client ping");
                assert_eq!(message.get_seq(), 1);
                message.set_caption("server pong".to_string());
                block_on(res.reply_message(&message))
                    .expect("[server] Failed to reply send_message");
                println!("[server] send_message({:?}) : {} ", message, addr);
                println!("[server] Done");
            },
            server::Config::new(1024 * 1024 * 16)
                .on_started(Box::new(move || {
                    tokio::spawn(async move {
                        test_client().await;
                    });
                }))
                .on_connect(Box::new(move |addr| match addr {
                    SocketAddr::V4(ip) => {
                        println!("on_connected({}) : {}", addr, ip.port() - 1);
                        TestContext {
                            number: ip.port() - 1,
                        }
                    }
                    SocketAddr::V6(ip) => {
                        println!("on_connect({}) : {}", addr, ip.port() - 1);
                        TestContext {
                            number: ip.port() - 1,
                        }
                    }
                }))
                .on_disconnect(Box::new(move |addr, ref ctx| {
                    let svc = svc.clone();
                    let x = match addr {
                        SocketAddr::V4(ip) => ip.port() - 1,
                        SocketAddr::V6(ip) => ip.port() - 1,
                    };
                    let y = ctx.number;
                    println!("on_disconnect({}) : {} != {}, {} = 10", addr, x, y, y);
                    assert_eq!(10, y);
                    svc.lock().unwrap().stop().unwrap();
                }))
                .on_error(Box::new(move |err, ctx| {
                    if let Some(ctx) = ctx {
                        println!("on_error : {}, {}", err, ctx.number);
                    } else {
                        println!("on_error : {}", err);
                    }
                })),
        )
        .await
        .unwrap();
    });
    match tokio::time::timeout(Duration::from_secs(30), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn context_test_with_thread() {
    struct TestContext {
        pub number: u16,
    }
    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client() {
            let client = wsmq_rs::client::connect("ws://127.0.0.1:65000")
                .await
                .expect("[client] Failed to client::connect");
            let mut message = protos::test::TestMessage::new();
            message.set_caption("client ping".to_string());
            message.set_seq(1);
            message.set_need_to_rely(true);
            println!("[client] send_message({:?})", message);
            let res = client
                .send_message(&message)
                .unwrap()
                .await
                .expect("Failed to send_message");
            let message = res
                .to_message::<protos::test::TestMessage>()
                .expect("[client] Failed to to_message");
            println!("[client] Message received ({:?})", message);
            assert_eq!(message.get_caption(), "server pong");
            assert_eq!(message.get_seq(), 1);
            println!("[client] Done");
        }
        wsmq_rs::server::run_with_config(
            "0.0.0.0:65000",
            move |addr, res, ctx| {
                let ctx = ctx.clone();
                tokio::runtime::Handle::current().spawn_blocking(move || {
                    let x = match addr {
                        SocketAddr::V4(ip) => ip.port() - 1,
                        SocketAddr::V6(ip) => ip.port() - 1,
                    };
                    let y = ctx.lock().unwrap().number;
                    println!("on_message({}) : {} = {}", addr, x, y);
                    assert_eq!(x, y);
                    ctx.lock().unwrap().number = 10;
                    println!("change ctx.number to {}", 10);

                    let mut message = res
                        .to_message::<protos::test::TestMessage>()
                        .expect("[server] Failed to to_message");
                    println!("[server] message received({:?}) : {} ", message, addr);
                    assert_eq!(message.get_caption(), "client ping");
                    assert_eq!(message.get_seq(), 1);
                    message.set_caption("server pong".to_string());
                    tokio::spawn(async move {
                        res.reply_message(&message)
                            .await
                            .expect("[server] Failed to reply send_message");
                        println!("[server] send_message({:?}) : {} ", message, addr);
                        println!("[server] Done");
                    });
                });
            },
            server::Config::new(1024 * 1024 * 16)
                .on_started(Box::new(move || {
                    tokio::spawn(async move {
                        test_client().await;
                    });
                }))
                .on_connect(Box::new(move |addr| match addr {
                    SocketAddr::V4(ip) => {
                        println!("on_connected({}) : {}", addr, ip.port() - 1);
                        Arc::new(Mutex::new(TestContext {
                            number: ip.port() - 1,
                        }))
                    }
                    SocketAddr::V6(ip) => {
                        println!("on_connect({}) : {}", addr, ip.port() - 1);
                        Arc::new(Mutex::new(TestContext {
                            number: ip.port() - 1,
                        }))
                    }
                }))
                .on_disconnect(Box::new(move |addr, ctx| {
                    let svc = svc.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let x = match addr {
                            SocketAddr::V4(ip) => ip.port() - 1,
                            SocketAddr::V6(ip) => ip.port() - 1,
                        };
                        let y = ctx.lock().unwrap().number;
                        println!("on_disconnect({}) : {} != {}, {} = 10", addr, x, y, y);
                        assert_eq!(10, y);
                        svc.lock().unwrap().stop().unwrap();
                    });
                }))
                .on_error(Box::new(move |err, ctx| {
                    if let Some(ctx) = ctx {
                        println!("on_error : {}, {}", err, ctx.lock().unwrap().number);
                    } else {
                        println!("on_error : {}", err);
                    }
                })),
        )
        .await
        .unwrap();
    });
    match tokio::time::timeout(Duration::from_secs(30), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}

#[tokio::test]
async fn complex_test() {
    let f = define_test_future!(|svc: Arc<Mutex<service_rs::service::Service>>, _| async {
        async fn test_client() {
            // connect to server
            match wsmq_rs::client::connect_with_config(
                "ws://127.0.0.1:65002",
                client::Config::new(1024 * 1024 * 4).on_error(Box::new(|err| {
                    println!("[client] on_error : {}", err);
                })),
            )
            .await
            {
                Ok(client) => {
                    // generate message
                    let mut msg = protos::test::TestMessage::new();
                    msg.set_caption("client ping".to_string());
                    // send only
                    msg.set_seq(0);
                    msg.set_need_to_rely(false);
                    println!("[client] send_message({:?})", msg);
                    client.send_message(&msg).unwrap();
                    // send and reply
                    msg.set_seq(1);
                    msg.set_need_to_rely(true);
                    println!("[client] send_message({:?})", msg);
                    match client.send_message(&msg) {
                        Ok(res) => {
                            println!("[client] Wait for reply message({:?})", msg);
                            match res.await {
                                Ok(res) => {
                                    match res.to_message::<protos::test::TestMessage>() {
                                        Ok(mut message) => {
                                            println!("[client] Message received ({:?})", message);
                                            assert_eq!(message.get_caption(), "server pong 1");
                                            assert_eq!(message.seq, 1000);
                                            message.set_caption("client ping 2".to_string());
                                            message.set_seq(2);
                                            message.set_need_to_rely(true);
                                            println!("[client] Reply send_message({:?})", message);
                                            match res.reply_message(&message).await {
                                                Ok(res) => {
                                                    match res.await {
                                                        Ok(res) => {
                                                            if let Ok(mut message) = res.to_message::<protos::test::TestMessage>() {
                                                        println!("[client] Message received ({:?})", message);
                                                        assert_eq!(message.get_caption(), "server pong 2");
                                                        assert_eq!(message.seq, 2000);
                                                        message.set_caption("client ping 3".to_string());
                                                        message.set_seq(3);
                                                        message.set_need_to_rely(true);
                                                        println!("[client] Reply send_message({:?})", message);
                                                        match res.reply_message(&message).await {
                                                            Ok(res) => {
                                                                match res.await {
                                                                    Ok(res) => {
                                                                        match res.to_message::<protos::test::TestMessage>() {
                                                                            Ok(mut message) => {
                                                                                println!("[client] Message received ({:?})", message);
                                                                                assert_eq!(message.get_caption(), "server pong 3");
                                                                                assert_eq!(message.seq, 3000);
                                                                                message.set_caption("client ping 4".to_string());
                                                                                message.set_seq(4);
                                                                                message.set_need_to_rely(true);
                                                                                println!("[client] Reply send_message({:?})", message);
                                                                                match res.reply_message(&message).await {
                                                                                    Ok(res) => {
                                                                                        match res.await {
                                                                                            Ok(res) => {
                                                                                                match res.to_message::<protos::test::TestMessage>() {
                                                                                                    Ok(message) => {
                                                                                                        println!("[client] On reply message({:?})", message);
                                                                                                        println!("[client] Done");
                                                                                                        //return; // or client.close();
                                                                                                    }
                                                                                                    Err(err) => {
                                                                                                        println!("[client] Failed to to_message : {}", err.cause())
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                            Err(err) => println!(
                                                                                                "[client] Failed to Response : {}",
                                                                                                err.cause()
                                                                                            ),
                                                                                        }
                                                                                    }
                                                                                    Err(err) => println!(
                                                                                        "[client] Failed to reply_message({:?}) : {}",
                                                                                        message,
                                                                                        err.cause()
                                                                                    ),
                                                                                }
                                                                            }
                                                                            Err(err) => {
                                                                                println!("[client] Failed to to_message : {}", err.cause())
                                                                            }
                                                                        }
                                                                    }
                                                                    Err(err) => println!(
                                                                        "[client] Failed to Response : {}",
                                                                        err.cause()
                                                                    ),
                                                                }
                                                            }
                                                            Err(err) => println!(
                                                                "[client] Failed to reply_message({:?}) : {}",
                                                                message,
                                                                err.cause()
                                                            ),
                                                        }
                                                    }
                                                        }
                                                        Err(err) => println!(
                                                            "[client] Failed to Response : {}",
                                                            err.cause()
                                                        ),
                                                    }
                                                }
                                                Err(err) => println!(
                                                    "[client] Failed to reply_message({:?}) : {}",
                                                    message,
                                                    err.cause()
                                                ),
                                            }
                                        }
                                        Err(err) => println!(
                                            "[client] Failed to to_message : {}",
                                            err.cause()
                                        ),
                                    }
                                }
                                Err(err) => {
                                    println!("[client] Failed to Response : {}", err.cause())
                                }
                            }
                        }
                        Err(err) => println!(
                            "[client] Failed to send_message({:?}) : {}",
                            msg,
                            err.cause()
                        ),
                    }
                }
                Err(err) => println!("[client] Failed to connect_with_config : {}", err.cause()),
            }
        }

        if let Err(err) = wsmq_rs::server::run_with_config(
            "0.0.0.0:65002",
            move |addr, res, ctx| match res.to_message::<protos::test::TestMessage>() {
                Ok(mut message) => {
                    println!("[server] On message : {}, {:?}", addr, message);
                    assert_eq!(message.seq, *ctx);
                    if message.need_to_rely {
                        let seq = message.seq;
                        message.set_caption(format!("server pong {}", *ctx));
                        message.set_seq(*ctx * 1000);
                        println!("[server] send_message({:?})", message);
                        match block_on(res.reply_message(&message)) {
                            Ok(_) => {
                                if seq == 4 {
                                    println!("[server] Done");
                                }
                            }
                            Err(err) => println!(
                                "[server] Failed to reply_message({:?}) : {}",
                                message,
                                err.cause()
                            ),
                        }
                    }
                    *ctx += 1;
                }
                Err(err) => println!("[server] Failed to to_message : {}", err.cause()),
            },
            server::Config::new(1024 * 1024 * 16)
                .on_started(Box::new(move || {
                    println!("[server] Started");
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        test_client().await;
                        svc.lock().unwrap().stop().unwrap();
                    });
                }))
                .on_connect(Box::new(move |addr| {
                    println!("[server] Connected : {}", addr);
                    0
                }))
                .on_disconnect(Box::new(move |addr, ctx| {
                    println!("[server] Disconnected : {}, {}", addr, ctx);
                }))
                .on_error(Box::new(move |err, _| {
                    println!("[server] Error : {}", err);
                })),
        )
        .await
        {
            println!("[server] Failed to run_with_config : {}", err.cause());
        }
    });
    match tokio::time::timeout(Duration::from_secs(20), f).await {
        Ok(_) => {}
        Err(err) => panic!("timeouted : {}", err),
    };
}
