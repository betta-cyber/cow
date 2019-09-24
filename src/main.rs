extern crate hyper;
extern crate futures;
extern crate tokio_core;
// #[macro_use]
// extern crate lazy_static;

use hyper::{Body, Request, Response, Server, Method, StatusCode, service::service_fn, header};
use hyper::rt::Future;
use regex::Regex;
use futures::{future, future::Either};
use std::net::{IpAddr, Ipv4Addr};
use http::Uri;
use percent_encoding::percent_decode_str;
use tokio::fs::File;
// use tokio_core::reactor::{Core, Handle};
// use tokio_core::net::TcpListener;
use std::{
    path::{Path, PathBuf},
    io,
};

// const PHRASE: &str = "Hello, World!";
type BoxFut = Box<dyn Future<Item=Response<Body>, Error=hyper::Error> + Send>;

// lazy_static! {
    // static ref PROXY_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
// }

static PROXY_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

fn parser_request(req: Request<Body>) -> BoxFut {

    let uri_path = req.uri().path();

    let pattern = Regex::new(r"^/api").unwrap();
    if pattern.is_match(uri_path) {
        return hyper_reverse_proxy::call(PROXY_IP, "http://127.0.0.1:8000", req)
    } else {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Box::new(future::ok(response))
    }
}

fn serve_static(req: &Request<Body>, root_dir: &PathBuf) -> impl Future<Item=Response<Body>, Error=hyper::Error> {
    let uri = req.uri().clone();
    let root_dir = root_dir.clone();

    try_dir_redirect(req, &root_dir).and_then(move |maybe_redir_resp| {
        if let Some(redir_resp) = maybe_redir_resp {
            return Either::A(future::ok(redir_resp));
        }

        if let Some(path) = local_path_with_maybe_index(&uri, &root_dir) {
            Either::B(
                File::open(path.clone())
                    .map_err(Error::from)
                    .and_then(move |file| respond_with_file(file, path)),
            )
        } else {
            Either::A(future::err(|_| ())
        }
    })
}


fn try_dir_redirect(req: &Request<Body>, root_dir: &PathBuf) -> impl Future<Item=Response<Body>, Error=hyper::Error> {
    if !req.uri().path().ends_with("/") {
        println!("path does not end with /");
        if let Some(path) = local_path_for_request(req.uri(), root_dir) {
            if path.is_dir() {
                let mut new_loc = req.uri().path().to_string();
                new_loc.push_str("/");
                if let Some(query) = req.uri().query() {
                    new_loc.push_str("?");
                    new_loc.push_str(query);
                }
                future::result(
                    Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, new_loc)
                        .body(Body::empty())
                        .map(Some)
                )
            } else {
                future::ok(None)
            }
        } else {
            future::err(hyper::Error())
        }
    } else {
        future::ok(None)
    }
}


// Map the request's URI to a local path
fn local_path_for_request(uri: &Uri, root_dir: &Path) -> Option<PathBuf> {
    let request_path = uri.path();

    if !request_path.starts_with("/") {
        return None;
    }

    let end = request_path.find('?').unwrap_or(request_path.len());
    let request_path = &request_path[0..end];

    // Convert %-encoding to actual values
    let decoded = percent_decode_str(&request_path);
    let request_path = if let Ok(p) = decoded.decode_utf8() {
        p
    } else {
        // FIXME: Error handling
        return None;
    };

    let mut path = root_dir.to_owned();
    if request_path.starts_with('/') {
        path.push(&request_path[1..]);
    } else {
        return None;
    }

    Some(path)
}


fn read_file(file: tokio::fs::File) -> impl Future<Item = Vec<u8>, Error = io::Error> {
    let buf: Vec<u8> = Vec::new();
    tokio::io::read_to_end(file, buf)
        .map_err(|_| ())
        .and_then(|(_read_handle, buf)| future::ok(buf))
}


// fn file_path_mime(file_path: &Path) -> mime::Mime {
    // let mime_type = match file_path.extension().and_then(std::ffi::OsStr::to_str) {
        // Some("html") => mime::TEXT_HTML,
        // Some("css") => mime::TEXT_CSS,
        // Some("js") => mime::TEXT_JAVASCRIPT,
        // Some("jpg") => mime::IMAGE_JPEG,
        // Some("md") => "text/markdown; charset=UTF-8"
            // .parse::<mime::Mime>()
            // .unwrap(),
        // Some("png") => mime::IMAGE_PNG,
        // Some("svg") => mime::IMAGE_SVG,
        // Some("wasm") => "application/wasm".parse::<mime::Mime>().unwrap(),
        // _ => mime::TEXT_PLAIN,
    // };
    // mime_type
// }


fn respond_with_file(file: tokio::fs::File, path: PathBuf) -> impl Future<Item = Response<Body>, Error = hyper::Error> {
    Some(read_file(file)).and_then(move |buf| {
        // let mime_type = file_path_mime(&path);
        let mime_type = "html";
    })
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, buf.len() as u64)
        .header(header::CONTENT_TYPE, mime_type.as_ref())
        .body(Body::from(buf))
        .map_err(|_| ())
    response
}


fn main() {
    let addr = ([0, 0, 0, 0], 3000).into();

    let server = Server::bind(&addr)
        .serve(|| service_fn(parser_request))
        .map_err(|e| eprintln!("server error: {}", e));

    hyper::rt::run(server);
}
