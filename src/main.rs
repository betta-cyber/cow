extern crate serde;
#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate serde_derive;

extern crate hyper;
extern crate futures;
extern crate config;
extern crate pretty_env_logger;
#[macro_use] extern crate log;
extern crate clap;

// #[macro_use]
// extern crate lazy_static;

use hyper::{Body, Request, Response, Server, StatusCode, service::{make_service_fn, service_fn}, header};
use hyper::server::conn::AddrStream;
use regex::Regex;
use futures::{future, future::Either, Future};
use std::net::{IpAddr, Ipv4Addr};
use http::Uri;
use percent_encoding::percent_decode_str;
use tokio::fs::File;
use clap::App;
use std::{
    path::{Path, PathBuf},
    error::Error as StdError,
    io,
    net::SocketAddr,
    collections::HashMap,
};

mod conf;
mod proxy;
use conf::Cowconfig;

// const PHRASE: &str = "Hello, World!";
type BoxFut = Box<dyn Future<Item=Response<Body>, Error=Error> + Send>;

// lazy_static! {
    // static ref PROXY_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
// }

static PROXY_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

// main function
// init config
// bind addr and listen port
fn main() {
    // new app
    let matches = App::new("cow")
        .version("0.1")
        .author("betta betta0801@gmail.com")
        .about("cow server")
        .args_from_usage(
            "-c, --config=[FILE] 'Sets a custom config file'"
            )
        .get_matches();

    let config_path = matches.value_of("config").unwrap_or("cow.toml");

    // init config
    let config = Cowconfig::new(&config_path).unwrap();
    if config.debug {
        std::env::set_var("RUST_LOG", "debug");
    } else {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let address_string = format!("{}:{}", config.address, config.port);
    let addr: SocketAddr = address_string.parse().unwrap();

    info!("init config");
    info!("listen on {}", address_string);
    debug!("{:#?}", config.server);

    // let addr = ([0, 0, 0, 0], 3000).into();
    // println!("{:#?}", addr);
    let make_svc = make_service_fn(move |socket: &AddrStream| {
        let remote_addr = socket.remote_addr();
        // println!("{:#?}", remote_addr);
        let config = config.clone();
        service_fn(move |req| {
            debug!("{:#?}", req);
            parser_request(req, &config).map_err(|e| {
                eprintln!("server error: {}", e);
                e
            })
        })
    });

    let server = Server::bind(&addr)
        .serve(make_svc)
        .map_err(|e| {
            eprintln!("server error: {}", e);
            ()
        });

    // let tls_cfg = {
        // // Load public certificate.
        // let certs = load_certs("examples/sample.pem")?;
        // // Load private key.
        // let key = load_private_key("examples/sample.rsa")?;
        // // Do not use client certificate authentication.
        // let mut cfg = rustls::ServerConfig::new(rustls::NoClientAuth::new());
        // // Select a certificate to use.
        // cfg.set_single_cert(certs, key)
            // .map_err(|e| error(format!("{}", e)))?;
        // sync::Arc::new(cfg)
    // };
    // let tls_acceptor = TlsAcceptor::from(tls_cfg);

    hyper::rt::run(server);
}


fn parser_request(req: Request<Body>, config: &Cowconfig) -> BoxFut {
    // get config
    let config = config.clone();
    // get uri path for location
    let uri_path = req.uri().path();
    let location: HashMap<String, String> = find_locatiton(config.server, uri_path);
    debug!("{:#?}", location);
    if location.contains_key("static_path") {
        let root_dir = PathBuf::from(config.root_dir);
        let res = serve_static(&req, &root_dir)
            .then(move |maybe_resp| {
                let re = match maybe_resp {
                    Ok(r) => r,
                    Err(_) => {
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("cow", "0.0.1")
                            .body(Body::from("not found"))
                            .unwrap()
                        // let mut response = Response::new(Body::empty());
                        // *response.status_mut() = StatusCode::NOT_FOUND;
                        // response
                    }
                };

                future::ok(re)
            });
            // .wait();
            // println!("{:#?}", a);
            Box::new(res)
    } else if location.contains_key("proxy_pass") {
        return proxy::proxy(PROXY_IP, "http://127.0.0.1:8000", req)
    } else {
        // bad error
        let re = Response::builder()
            .status(StatusCode::OK)
            .header("cow", "0.0.1")
            .body(Body::from("not found"))
            .unwrap();
        let res = future::ok(re);
        Box::new(res)
    }
}

// find uri location and return location config
fn find_locatiton(config: Vec<conf::Server>, uri: &str) -> HashMap<String, String>{
    for c in config.iter() {
        let location = c.location.clone();
        let s = location.get("pattern").unwrap().clone();
        let pattern = Regex::new(&s[..]).unwrap();
        if pattern.is_match(uri) {
            return location;
        }
    }
    HashMap::new()
}

// serve static file
// try to get dir redirect in progress
// check if redir or path or not found
fn serve_static(req: &Request<Body>, root_dir: &PathBuf) -> impl Future<Item=Response<Body>, Error=Error> {
    let uri = req.uri().clone();
    let root_dir = root_dir.clone();

    try_dir_redirect(req, &root_dir).and_then(move |maybe_redir_resp| {
        if let Some(redir_resp) = maybe_redir_resp {
            return Either::A(future::ok(redir_resp));
        }

        if let Some(path) = local_path_with_maybe_index(&uri, &root_dir) {
            return Either::B(
                // open file and make response with file
                File::open(path.clone())
                    .map_err(Error::from)
                    .and_then(move |file| respond_with_file(file, path)),
            )
        } else {
            return Either::A(future::err(Error::UrlToPath))
        }
    })
}



// try to get the path index file
fn local_path_with_maybe_index(uri: &Uri, root_dir: &Path) -> Option<PathBuf> {
    local_path_for_request(uri, root_dir).map(|mut p: PathBuf| {
        if p.is_dir() {
            p.push("index.html");
        } else {
            // println!("trying path as from URL");
        }
        p
    })
}


fn try_dir_redirect(req: &Request<Body>, root_dir: &PathBuf) -> impl Future<Item=Option<Response<Body>>, Error=Error> {
    if !req.uri().path().ends_with("/") {
        // println!("path does not end with /");
        // if path does not end of "/", it means does not a path
        // if might be a file
        if let Some(path) = local_path_for_request(req.uri(), root_dir) {
            // println!("{:#?}", path.is_dir());
            if path.is_dir() {
                let mut new_loc = req.uri().path().to_string();
                new_loc.push_str("/");
                if let Some(query) = req.uri().query() {
                    new_loc.push_str("?");
                    new_loc.push_str(query);
                }
                // println!("{:#?}", new_loc);
                future::result(
                    Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, new_loc)
                        .body(Body::empty())
                        .map(Some)
                        .map_err(Error::from),
                )
            } else {
                future::ok(None)
            }
        } else {
            future::err(Error::UrlToPath)
        }
    } else {
        future::ok(None)
    }
}


// Map the request's URI to a local path
fn local_path_for_request(uri: &Uri, root_dir: &Path) -> Option<PathBuf> {
    let request_path = uri.path();

    // check uri start with /
    if !request_path.starts_with("/") {
        return None;
    }

    // split when find ? in uri
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


fn read_file(file: tokio::fs::File) -> impl Future<Item = Vec<u8>, Error = Error> {
    let buf: Vec<u8> = Vec::new();
    // use tokio io to read the file
    tokio::io::read_to_end(file, buf)
        .map_err(Error::Io)
        .and_then(|(_read_handle, buf)| future::ok(buf))
}


// get the file ext mime type
fn file_path_mime(file_path: &Path) -> mime::Mime {
    let mime_type = match file_path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("html") => mime::TEXT_HTML,
        Some("css") => mime::TEXT_CSS,
        Some("js") => mime::TEXT_JAVASCRIPT,
        Some("jpg") => mime::IMAGE_JPEG,
        Some("md") => "text/markdown; charset=UTF-8".parse::<mime::Mime>().unwrap(),
        Some("png") => mime::IMAGE_PNG,
        Some("svg") => mime::IMAGE_SVG,
        Some("wasm") => "application/wasm".parse::<mime::Mime>().unwrap(),
        _ => mime::TEXT_PLAIN,
    };
    mime_type
}

// build response
fn respond_with_file(file: tokio::fs::File, path: PathBuf) -> impl Future<Item=Response<Body>, Error=Error> {
    read_file(file).and_then(move |buf| {
        let mime_type = file_path_mime(&path);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, buf.len() as u64)
            .header(header::CONTENT_TYPE, mime_type.as_ref())
            .header("cow", "0.0.1")
            .body(Body::from(buf))
            .map_err(Error::from)
    })
}

fn make_error_response(e: Error) -> impl Future<Item = Response<Body>, Error = Error> {
    match e {
        Error::Io(e) => Either::A(make_io_error_response(e)),
        e => Either::B(make_internal_server_error_response(e)),
    }
}

// return 500
fn make_io_error_response(error: io::Error) -> impl Future<Item = Response<Body>, Error = Error> {
    match error.kind() {
        io::ErrorKind::NotFound => {
            debug!("{}", error);
            Either::A(make_error_response_from_code(StatusCode::NOT_FOUND))
        }
        _ => Either::B(make_internal_server_error_response(Error::Io(error))),
    }
}

// convert an error into a 500 internal server error, and log it.
fn make_internal_server_error_response(err: Error) -> impl Future<Item = Response<Body>, Error = Error> {
    make_error_response_from_code(StatusCode::INTERNAL_SERVER_ERROR)
}


/// Make an error response given an HTTP status code.
fn make_error_response_from_code(status: StatusCode) -> impl Future<Item = Response<Body>, Error = Error> {
    future::result({ render_error_html(status) })
        .and_then(move |body| html_str_to_response(body, status))
}


/// Make an HTTP response from a HTML string.
fn html_str_to_response(body: String, status: StatusCode) -> Result<Response<Body>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, body.len())
        .header(header::CONTENT_TYPE, mime::TEXT_HTML.as_ref())
        .body(Body::from(body))
        .map_err(Error::from)
}


#[derive(Debug, Display)]
pub enum Error {
    // blanket "pass-through" error types
    #[display(fmt = "HTTP error")]
    Http(http::Error),

    #[display(fmt = "I/O error")]
    Io(io::Error),

    // custom "semantic" error types
    #[display(fmt = "failed to parse IP address")]
    AddrParse(std::net::AddrParseError),

    #[display(fmt = "markdown is not UTF-8")]
    MarkdownUtf8,

    #[display(fmt = "failed to strip prefix in directory listing")]
    StripPrefixInDirList(std::path::StripPrefixError),

    // #[display(fmt = "failed to render template")]
    // TemplateRender(handlebars::TemplateRenderError),

    #[display(fmt = "failed to convert URL to local file path")]
    UrlToPath,

    #[display(fmt = "formatting error while creating directory listing")]
    WriteInDirList(std::fmt::Error),
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        use Error::*;

        match self {
            Http(e) => Some(e),
            Io(e) => Some(e),
            AddrParse(e) => Some(e),
            MarkdownUtf8 => None,
            StripPrefixInDirList(e) => Some(e),
            // TemplateRender(e) => Some(e),
            UrlToPath => None,
            WriteInDirList(e) => Some(e),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Error {
        Error::Http(e)
    }
}
