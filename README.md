# wsmq-rs

A simple websocket messaging library based on protocol buffers.

## Example

### Basic

#### Server

```rust
wsmq_rs::server::run("0.0.0.0:8080", |addr, res, _| {
    let mut recieved = res.to_vec().clone();
    tokio::spawn(async move {
        recieved.extend(&addr.port().to_le_bytes());
        res.reply(recieved).await;
    });
})
.await
.unwrap();
```

#### Client

```rust
let client = wsmq_rs::client::connect("ws://127.0.0.1:8080")
    .await
    .unwrap();
client.send(vec![1, 2, 3, 4, 5]).unwrap();

let res = client.send(vec![1, 2, 3, 4, 5]).unwrap().await.unwrap();
let recieved = res.to_vec();
println!("{:?}", recieved);
```

---

### With protocol buffers
#### Protocol buffers
```proto
syntax = "proto3";

message RequestMessage {
    string text = 1;
}
message ResponseMessage {
    string text = 1;
    bytes payload = 2;
}
```

#### Server

```rust
wsmq_rs::server::run("0.0.0.0:8080", |addr, res, _| {
    let mut message = res.to_message::<RequestMessage>().expect("[server] Failed to to_message");
    tokio::spawn(async move {
        let mut response_message = ResponseMessage::new();
        response_message.set_text("response text".to_string());
        response_message.set_payload(message.get_text().as_bytes().to_vec());
        res.reply_message(&response_message).await;
    });
})
.await
.expect("[server] Failed to run");
```

#### Client

```rust
let client = wsmq_rs::client::connect("ws://127.0.0.1:8080")
    .await
    .expect("[client] Failed to connect");

let mut message = RequestMessage::new();
message.set_text("request text".to_string());

let res = client.send_message(&message)
    .expect("[client] Failed to send_message");
    .await
    .expect("[client] Failed to send_message");

let message = res.to_message::<ResponseMessage>()
    .expect("[client] Failed to to_message");

println!("{:?}", message);
```

---

### With config

#### Server with config

```rust
wsmq_rs::server::run_with_config(
    "0.0.0.0:8080",
    |addr, res, _| {
        let mut recieved = res.to_vec().clone();
        tokio::spawn(async move {
            recieved.extend(&addr.port().to_le_bytes());
            res.reply(recieved).await;
        });
    },
    wsmq_rs::server::Config::<()>::new().set_bandwidth(1024 * 1024 * 6),
)
.await
.unwrap();
```

#### Client with config

```rust
let client = wsmq_rs::client::connect_with_config(
    "ws://127.0.0.1:8080",
    wsmq_rs::client::Config::new().set_bandwidth(1024 * 1024 * 6),
)
.await
.unwrap();
client.send(vec![1, 2, 3, 4, 5]).unwrap();

let res = client.send(vec![1, 2, 3, 4, 5]).unwrap().await.unwrap();
let recieved = res.to_vec();
println!("{:?}", recieved);
```

---

### With context

#### Server with context

```rust
#[derive(Debug)]
struct Context {
    port: u16,
    sent: usize,
    recieved: usize,
}

wsmq_rs::server::run_with_config(
    "0.0.0.0:8080",
    |addr, res, context| {
        context.recieved += 1;
        let mut recieved = res.to_vec().clone();
        recieved.extend(&addr.port().to_le_bytes());
        if let Ok(_) = block_on(res.reply(recieved)) {
            context.sent += 1;
        }
    },
    wsmq_rs::server::Config::new()
        set_bandwidth(1024),
        .on_connect(Box::new(|addr| {
            println!("connected ({})", addr);
            Context {
                port: addr.port(),
                sent: 0,
                recieved: 0,
            }
        }))
        .on_disconnect(Box::new(|addr, context| {
            assert_eq!(addr.port(), context.port);
            println!("disconnected({}): {:?}", addr, context);
        }))
        .on_started(Box::new(|| {
            println!("started");
        }))
        .on_error(Box::new(|err, ctx| {
            if let Some(ctx) = ctx {
                println!("error({:?}) : {}", ctx, err);
            } else {
                println!("error : {}", err);
            }
        }))
        .on_progress(Box::new(|pctx, ctx| {
            println!("progress ({:?}) : {:?}", ctx, pctx);
        })),
)
.await
.unwrap();
```

#### Client with context

```rust
let client = wsmq_rs::client::connect_with_config(
    "ws://127.0.0.1:8080",
    wsmq_rs::client::Config::new().set_bandwidth(1024),
)
.await
.unwrap();
client.send(vec![1, 2, 3, 4, 5]).unwrap();
let res = client.send(vec![1, 2, 3, 4, 5]).unwrap().await.unwrap();
let recieved = res.to_vec();
println!("{:?}", recieved);
```
