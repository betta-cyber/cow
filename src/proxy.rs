use hyper::Body;
use std::net::IpAddr;
use std::str::FromStr;
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Request, Response, Client, Uri, StatusCode};
use futures::future::{self, Future};
use lazy_static::lazy_static;

// use crate's error
use crate::Error;

type BoxFut = Box<dyn Future<Item=Response<Body>, Error=Error> + Send>;

pub fn proxy(client_ip: IpAddr, forword_uri: &str, request: Request<Body>) -> BoxFut {

    let proxy_request = create_proxy_request(client_ip, forword_uri, request);
    let client = Client::new();

    let response = client.request(proxy_request).then(|response| {
        let proxy_response = match response {
            Ok(response) => create_response(response),
            Err(error) => {
                // todo
                // return  500 template
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(error.to_string()))
                    .unwrap()
            },
        };
        future::ok(proxy_response)
    });


    Box::new(response)
}


fn create_proxy_request(client_ip: IpAddr, forward_url: &str, mut request: Request<Body>) -> Request<Body> {
    *request.headers_mut() = remove_hop_headers(request.headers());
    *request.uri_mut() = forward_uri(forward_url, &request);

    let x_forwarded_for_header = "x-forwarded-for";

    match request.headers_mut().entry(x_forwarded_for_header) {
        Ok(header_entry) => {
            match header_entry {
                hyper::header::Entry::Vacant(entry) => {
                    let addr = format!("{}", client_ip);
                    entry.insert(addr.parse().unwrap());
                },

                hyper::header::Entry::Occupied(mut entry) => {
                    let addr = format!("{}, {}", entry.get().to_str().unwrap(), client_ip);
                    entry.insert(addr.parse().unwrap());
                }
            }
        }

        // shouldn't happen...
        Err(_) => panic!("Invalid header name: {}", x_forwarded_for_header),
    }
    request
}

// generate forword uri
fn forward_uri(forward_url: &str, req: &Request<Body>) -> Uri {
    let forward_uri = match req.uri().query() {
        Some(query) => format!("{}{}?{}", forward_url, req.uri().path(), query),
        None => format!("{}{}", forward_url, req.uri().path()),
    };

    Uri::from_str(forward_uri.as_str()).unwrap()
}

// remove hop header
fn remove_hop_headers(headers: &HeaderMap<HeaderValue>) -> HeaderMap<HeaderValue> {
    let mut result = HeaderMap::new();
    for (k, v) in headers.iter() {
        if !is_hop_header(k.as_str()) {
            result.insert(k.clone(), v.clone());
        }
    }
    result
}

// check hop header
fn is_hop_header(name: &str) -> bool {
    use unicase::Ascii;

    // A list of the headers, using `unicase` to help us compare without
    // worrying about the case, and `lazy_static!` to prevent reallocation
    // of the vector.
    lazy_static! {
        static ref HOP_HEADERS: Vec<Ascii<&'static str>> = vec![
            Ascii::new("Connection"),
            Ascii::new("Keep-Alive"),
            Ascii::new("Proxy-Authenticate"),
            Ascii::new("Proxy-Authorization"),
            Ascii::new("Te"),
            Ascii::new("Trailers"),
            Ascii::new("Transfer-Encoding"),
            Ascii::new("Upgrade"),
        ];
    }

    HOP_HEADERS.iter().any(|h| h == &name)
}

fn create_response(mut response: Response<Body>) -> Response<Body> {
    *response.headers_mut() = remove_hop_headers(response.headers());
    response
}
