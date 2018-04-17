#![feature(conservative_impl_trait)]

extern crate failure;
extern crate futures;
extern crate rmp_rpc;
extern crate rmp_serde;
extern crate rmpv;
extern crate serde;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_uds;

#[macro_use] extern crate failure_derive;

use rmp_rpc::{Client as RpcClient};
use rmpv::Value;
use std::io;
use std::path::Path;
use tokio_core::reactor;
use tokio_uds::UnixStream;

#[derive(Debug, Fail)]
pub enum Error {
    IoError(io::Error),
    NvimReturnedError(Value),
    UnexpectedReturnType(Value),
    ConnectionClosed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        std::fmt::Debug::fmt(self, f)
    }
}

pub struct Map {
    vec: Vec<(Value, Value)>,
}

impl Map {
    pub fn as_slice(&self) -> &[(Value, Value)] {
        &self.vec
    }
}

impl From<Map> for Value {
    fn from(m: Map) -> Value {
        Value::Map(m.vec)
    }
}

pub struct NvimClient {
    client: RpcClient,
}

impl NvimClient {
    pub fn from_unix_socket<P: AsRef<Path>>(path: P, handle: &reactor::Handle) -> Result<NvimClient, io::Error> {
        let stream = UnixStream::connect(path, handle)?;
        Ok(NvimClient {
            client: RpcClient::new(stream, handle),
        })
    }
}

trait FromValue<'client> {
    fn from_value(v: Value, client: &'client RpcClient) -> Result<Self, Error> where Self: Sized;
}

macro_rules! impl_from_value {
    ($ty:ty, $val:ident, $pat:pat = $val_expr:expr => $ret_expr:expr) => {
        #[allow(unreachable_patterns)]
        impl<'client> FromValue<'client> for $ty {
            fn from_value($val: Value, _client: &'client RpcClient) -> Result<$ty, Error> {
                match $val_expr {
                    $pat => Ok($ret_expr),
                    _ => Err(Error::UnexpectedReturnType($val))
                }
            }
        }
    }
}

impl_from_value!((), v, Value::Nil = v => ());
impl_from_value!(bool, v, Value::Boolean(b) = v => b);
impl_from_value!(Value, v, v = v => v);
impl_from_value!(String, v, Value::String(s) = v => s.to_string());
impl_from_value!(i64, v, Some(i) = v.as_i64() => i);
impl_from_value!(Map, v, Value::Map(m) = v => Map { vec: m });

impl<'client, Inner> FromValue<'client> for Vec<Inner>
where Inner: FromValue<'client>
{
    fn from_value(v: Value, client: &'client RpcClient) -> Result<Vec<Inner>, Error> {
        match v {
            Value::Array(a) => {
                let ret = a.into_iter()
                    .map(|x| Inner::from_value(x, client))
                    .collect::<Result<Vec<Inner>, Error>>();
                Ok(ret?)
            }
            _ => Err(Error::UnexpectedReturnType(v))
        }
    }
}

impl<'client, A, B> FromValue<'client> for (A, B)
where
A: FromValue<'client>,
B: FromValue<'client>,
{
    fn from_value(v: Value, client: &'client RpcClient) -> Result<(A, B), Error> {
        match v {
            Value::Array(ref a) if a.len() == 2 => {
                let x = A::from_value(a[0].clone(), client)?;
                let y = B::from_value(a[1].clone(), client)?;
                Ok((x, y))
            }
            _ => Err(Error::UnexpectedReturnType(v))
        }
    }
}

// This is like Into<Value>, but the point is that because neither Value or Into (or From) was
// defined in this crate, we run into coherence issues.
trait IntoValue {
    fn into_value(self) -> Value;
}

// We can't do impl<T: Into<Value>> IntoValue for T because of coherence, so we'll do them
// one-by-one.
macro_rules! impl_into_value {
    ($ty:ty) => {
        impl IntoValue for $ty {
            fn into_value(self) -> Value { self.into() }
        }
    }
}

impl_into_value!(Map);
impl_into_value!(Value);
impl_into_value!(String);
impl_into_value!(bool);
impl_into_value!(i64);

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::Array(self.into_iter().map(|v| v.into_value()).collect())
    }
}

impl<S: IntoValue, T: IntoValue> IntoValue for (S, T) {
    fn into_value(self) -> Value {
        Value::Array(vec![self.0.into_value(), self.1.into_value()])
    }
}

fn convert_ret<'client, Ret>(nvim_ret: Result<Value, Value>, client: &'client RpcClient)
-> Result<Ret, Error>
where Ret: FromValue<'client>
{
    match nvim_ret {
        Ok(x) => Ret::from_value(x, client),
        Err(x) => Err(Error::NvimReturnedError(x)),
    }
}

// A macro for generating a wrapper for a neovim api type.
macro_rules! nvim_type {
    ($ty_name: ident) => {
        pub struct $ty_name<'client> {
            client: &'client RpcClient,
            data: Value,
        }

        impl<'client> FromValue<'client> for $ty_name<'client> {
            fn from_value(data: Value, client: &'client RpcClient) -> Result<$ty_name<'client>, Error> {
                Ok($ty_name {
                    client,
                    data,
                })
            }
        }

        impl<'client> IntoValue for $ty_name<'client> {
            fn into_value(self) -> Value {
                self.data
            }
        }
    }
}

// A macro for generating a typed wrapper of a neovim api method.
macro_rules! nvim_api_method {
    ($fn_name:ident, $nvim_fn_name:expr, $( $arg_name:ident : $arg_ty:ty ),*; $ret_ty:ty) => {
        pub fn $fn_name(&'client self, $( $arg_name : $arg_ty ),*) -> impl Future<Item = $ret_ty, Error = Error> + 'client {
            self.client.request($nvim_fn_name, &[ self.data.clone(), $( $arg_name.into_value() ),* ])
                .map_err(|_e| Error::ConnectionClosed)
                .and_then(move |v| convert_ret(v, self.client))
        }
    }
}

// A macro for generating a typed wrapper of a neovim api function.
//
// Here's a sample of the output of this macro:
//
// pub fn get_color_by_name(&self, name: &str) -> impl Future<Item = Integer, Error = Error> {
//     self.client.request("nvim_get_color_by_name", &[name.into()])
//         .map_err(|_e| Error::ConnectionClosed)
//         .and_then(convert_ret)
// }
//
// pub fn get_current_buf<'conn>(&'conn self) -> Buffer<'conn, T> {
//     self.client.request("nvim_get_current_buf", &[])
//         .map_err(|_e| Error::ConnectionClosed)
//         .map(|v| Buffer { conn: self, data: v })
// }
macro_rules! nvim_api_function {
    ($fn_name:ident, $nvim_fn_name:expr, $( $arg_name:ident : $arg_ty:ty ),*; $ret_ty:ty) => {
        pub fn $fn_name<'client>(&'client self, $( $arg_name : $arg_ty ),*) -> impl Future<Item = $ret_ty, Error = Error> + 'client {
            self.client.request($nvim_fn_name, &[ $( $arg_name.into_value() ),* ])
                .map_err(|_e| Error::ConnectionClosed)
                .and_then(move |v| convert_ret(v, &self.client))
        }
    }
}

mod api_autogen;

