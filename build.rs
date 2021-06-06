extern crate protobuf_codegen_pure;

fn main() {
    protobuf_codegen_pure::Codegen::new()
        .out_dir("./tests/protos")
        .customize(protobuf_codegen_pure::Customize {
            gen_mod_rs: Some(true),
            ..Default::default()
        })
        .inputs(&["./tests/protos/basic.proto"])
        .include("./tests/protos")
        .run()
        .expect("Codegen failed.");

    protobuf_codegen_pure::Codegen::new()
        .out_dir("./src/protos")
        .customize(protobuf_codegen_pure::Customize {
            gen_mod_rs: Some(true),
            ..Default::default()
        })
        .inputs(&["./protos/status.proto", "./protos/message.proto"])
        .include("./protos")
        .run()
        .expect("Codegen failed.");
}
