// This file is generated by rust-protobuf 2.23.0. Do not edit
// @generated

// https://github.com/rust-lang/rust-clippy/issues/702
#![allow(unknown_lints)]
#![allow(clippy::all)]

#![allow(unused_attributes)]
#![cfg_attr(rustfmt, rustfmt::skip)]

#![allow(box_pointers)]
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(trivial_casts)]
#![allow(unused_imports)]
#![allow(unused_results)]
//! Generated file from `status.proto`

/// Generated files are compatible only with the same version
/// of protobuf runtime.
// const _PROTOBUF_VERSION_CHECK: () = ::protobuf::VERSION_2_23_0;

#[derive(Clone,PartialEq,Eq,Debug,Hash)]
pub enum Status {
    OK = 200,
    NOT_FOUNT = 404,
    UNAUTHORIZED = 406,
    INVALID_TOKEN = 497,
    ACCESS_TOKEN_EXPIRED = 498,
    INTERNAL_ERROR = 500,
}

impl ::protobuf::ProtobufEnum for Status {
    fn value(&self) -> i32 {
        *self as i32
    }

    fn from_i32(value: i32) -> ::std::option::Option<Status> {
        match value {
            200 => ::std::option::Option::Some(Status::OK),
            404 => ::std::option::Option::Some(Status::NOT_FOUNT),
            406 => ::std::option::Option::Some(Status::UNAUTHORIZED),
            497 => ::std::option::Option::Some(Status::INVALID_TOKEN),
            498 => ::std::option::Option::Some(Status::ACCESS_TOKEN_EXPIRED),
            500 => ::std::option::Option::Some(Status::INTERNAL_ERROR),
            _ => ::std::option::Option::None
        }
    }

    fn values() -> &'static [Self] {
        static values: &'static [Status] = &[
            Status::OK,
            Status::NOT_FOUNT,
            Status::UNAUTHORIZED,
            Status::INVALID_TOKEN,
            Status::ACCESS_TOKEN_EXPIRED,
            Status::INTERNAL_ERROR,
        ];
        values
    }

    fn enum_descriptor_static() -> &'static ::protobuf::reflect::EnumDescriptor {
        static descriptor: ::protobuf::rt::LazyV2<::protobuf::reflect::EnumDescriptor> = ::protobuf::rt::LazyV2::INIT;
        descriptor.get(|| {
            ::protobuf::reflect::EnumDescriptor::new_pb_name::<Status>("Status", file_descriptor_proto())
        })
    }
}

impl ::std::marker::Copy for Status {
}

// Note, `Default` is implemented although default value is not 0
impl ::std::default::Default for Status {
    fn default() -> Self {
        Status::OK
    }
}

impl ::protobuf::reflect::ProtobufValue for Status {
    fn as_ref(&self) -> ::protobuf::reflect::ReflectValueRef {
        ::protobuf::reflect::ReflectValueRef::Enum(::protobuf::ProtobufEnum::descriptor(self))
    }
}

static file_descriptor_proto_data: &'static [u8] = b"\
    \n\x0cstatus.proto*z\n\x06Status\x12\x07\n\x02OK\x10\xc8\x01\x12\x0e\n\t\
    NOT_FOUNT\x10\x94\x03\x12\x11\n\x0cUNAUTHORIZED\x10\x96\x03\x12\x12\n\rI\
    NVALID_TOKEN\x10\xf1\x03\x12\x19\n\x14ACCESS_TOKEN_EXPIRED\x10\xf2\x03\
    \x12\x13\n\x0eINTERNAL_ERROR\x10\xf4\x03\x1a\0B\0b\x06proto3\
";

static file_descriptor_proto_lazy: ::protobuf::rt::LazyV2<::protobuf::descriptor::FileDescriptorProto> = ::protobuf::rt::LazyV2::INIT;

fn parse_descriptor_proto() -> ::protobuf::descriptor::FileDescriptorProto {
    ::protobuf::Message::parse_from_bytes(file_descriptor_proto_data).unwrap()
}

pub fn file_descriptor_proto() -> &'static ::protobuf::descriptor::FileDescriptorProto {
    file_descriptor_proto_lazy.get(|| {
        parse_descriptor_proto()
    })
}
