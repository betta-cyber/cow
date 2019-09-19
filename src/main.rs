#[macro_use]
extern crate log;
extern crate futures;
extern crate pretty_env_logger;
extern crate tokio;

use futures::future::{done, ok};
use futures::{Future, Stream};
use tokio::io::{self as tio, AsyncRead};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::copy;

use std::error;
use std::fmt;
use std::io;

fn accept_request(socket: TcpStream) -> impl Future<Item = (), Error = ()> + 'static + Send {
    futures::lazy(move || match socket.peer_addr() {
        Ok(peer) => {
            info!("Tcp connection [{:?}] connected to server", peer);
            Ok((socket, peer))
        }
        Err(err) => {
            error!("Fetch peer address failed: {:?}", err);
            Err(())
        }
    }).and_then(move |(socket, peer)| {
            let buf = Vec::new();
            let svc_fut = tio::read_to_end(socket, buf)
                .and_then(|(socket, buf)| {
                    println!("{:#?}", buf);
                    tio::write_all(socket, buf)
                })
                .then(|_| Ok(()));

            tokio::spawn(svc_fut);
            ok(())
        })
}

fn server_fut(listener: TcpListener) -> impl Future<Item = (), Error = ()> + 'static + Send {
    listener
        .incoming()
        .for_each(|socket| {
            // Split up the handle the socket
            tokio::spawn(accept_request(socket));
            Ok(())
        })
        .map_err(|err| {
            error!("Accept connection failed: {:?}", err);
        })
}

fn run() -> Result<(), io::Error> {
    let addr = "0.0.0.0:1234".parse().unwrap();
    info!("Listening on {:?}", addr);

    let listener = TcpListener::bind(&addr)?;
    let server_fut = server_fut(listener);

    tokio::run(server_fut);
    Ok(())
}

fn print<T: fmt::Debug, E: error::Error>(result: Result<T, E>) {
    match result {
        Ok(any) => info!("Result: {:?}", any),
        Err(err) => error!("Error: {:?}", err),
    }
}

fn init() {
    pretty_env_logger::init();
}

fn main() {
    init();
    print(run());
}
