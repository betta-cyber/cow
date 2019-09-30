extern crate serde;
#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate serde_derive;

extern crate hyper;
extern crate futures;
extern crate tokio_core;
extern crate config;
extern crate pretty_env_logger;
#[macro_use] extern crate log;

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
// use tokio_core::reactor::{Core, Handle};
// use tokio_core::net::TcpListener;
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
    // init config
    let config = Cowconfig::new().unwrap();
    if config.debug {
        std::env::set_var("RUST_LOG", "debug");
    }
    pretty_env_logger::init();

    let address_string = format!("{}:{}", config.address, config.port);
    let addr: SocketAddr = address_string.parse().unwrap();

    info!("init config");
    debug!("{:#?}", config.server);

    // let addr = ([0, 0, 0, 0], 3000).into();
    // println!("{:#?}", addr);
    let make_svc = make_service_fn(move |socket: &AddrStream| {
        let remote_addr = socket.remote_addr();
        // println!("{:#?}", remote_addr);
        let config = config.clone();
        service_fn(move |req| {
            info!("{:#?}", req);
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


times in msec
 clock   self+sourced   self:  sourced script
 clock   elapsed:              other lines

000.007  000.007: --- VIM STARTING ---
000.127  000.120: Allocated generic buffers
000.268  000.141: locale set
000.277  000.009: window checked
000.800  000.523: inits 1
000.837  000.037: parsing arguments
000.838  000.001: expanding arguments
000.860  000.022: shell init
001.352  000.492: Termcap init
001.381  000.029: inits 2
001.531  000.150: init highlight
001.964  000.313  000.313: sourcing /usr/share/vim/vim80/debian.vim
003.136  000.980  000.980: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
003.285  001.192  000.212: sourcing /usr/share/vim/vim80/syntax/synload.vim
010.874  007.530  007.530: sourcing /usr/share/vim/vim80/filetype.vim
010.933  008.906  000.184: sourcing /usr/share/vim/vim80/syntax/syntax.vim
010.964  009.387  000.168: sourcing $VIM/vimrc
011.492  000.361  000.361: sourcing /usr/share/vim/vim80/syntax/nosyntax.vim
011.836  000.217  000.217: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
011.962  000.414  000.197: sourcing /usr/share/vim/vim80/syntax/synload.vim
011.995  000.926  000.151: sourcing /usr/share/vim/vim80/syntax/syntax.vim
012.758  000.535  000.535: sourcing /usr/share/vim/vim80/ftoff.vim
015.101  001.953  001.953: sourcing /home/betta/.vim/autoload/plug.vim
021.845  000.103  000.103: sourcing /home/betta/.vim/bundle/vim-markdown/ftdetect/markdown.vim
031.940  000.040  000.040: sourcing /home/betta/.vim/bundle/ultisnips/ftdetect/snippets.vim
032.091  000.053  000.053: sourcing /home/betta/.vim/bundle/vim-openresty/ftdetect/nginx.vim
032.234  000.071  000.071: sourcing /home/betta/.vim/bundle/rust.vim/ftdetect/rust.vim
032.561  000.018  000.018: sourcing /home/betta/.vim/bundle/vim-fugitive/ftdetect/fugitive.vim
032.968  000.083  000.083: sourcing /home/betta/.vim/bundle/Vim-Jinja2-Syntax/ftdetect/jinja.vim
033.382  000.280  000.280: sourcing /home/betta/.vim/bundle/vim-javascript/ftdetect/javascript.vim
033.625  008.843  008.298: sourcing /usr/share/vim/vim80/filetype.vim
034.265  000.069  000.069: sourcing /usr/share/vim/vim80/ftplugin.vim
034.850  000.063  000.063: sourcing /usr/share/vim/vim80/indent.vim
036.269  024.229  012.663: sourcing /home/betta/.vimrc.bundles
036.812  000.022  000.022: sourcing /usr/share/vim/vim80/filetype.vim
037.430  000.021  000.021: sourcing /usr/share/vim/vim80/ftplugin.vim
037.978  000.019  000.019: sourcing /usr/share/vim/vim80/indent.vim
038.473  000.016  000.016: sourcing /usr/share/vim/vim80/filetype.vim
038.992  000.016  000.016: sourcing /usr/share/vim/vim80/filetype.vim
039.507  000.015  000.015: sourcing /usr/share/vim/vim80/indent.vim
040.036  000.018  000.018: sourcing /usr/share/vim/vim80/filetype.vim
040.550  000.013  000.013: sourcing /usr/share/vim/vim80/ftplugin.vim
041.125  000.021  000.021: sourcing /usr/share/vim/vim80/filetype.vim
041.667  000.014  000.014: sourcing /usr/share/vim/vim80/ftplugin.vim
042.163  000.012  000.012: sourcing /usr/share/vim/vim80/indent.vim
044.777  000.295  000.295: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
046.268  000.281  000.281: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
047.436  000.266  000.266: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
048.390  000.269  000.269: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
049.279  004.054  003.238: sourcing /home/betta/.vim/bundle/molokai/colors/molokai.vim
049.352  038.344  008.653: sourcing $HOME/.vimrc
049.357  000.095: sourcing vimrc file(s)
049.941  000.034  000.034: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/autoloclist.vim
049.989  000.019  000.019: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/balloons.vim
050.019  000.013  000.013: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/checker.vim
050.048  000.014  000.014: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/cursor.vim
050.127  000.015  000.015: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/highlighting.vim
050.161  000.017  000.017: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/loclist.vim
050.272  000.020  000.020: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/modemap.vim
050.309  000.019  000.019: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/notifiers.vim
050.338  000.013  000.013: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/registry.vim
050.365  000.012  000.012: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/signs.vim
051.418  000.810  000.810: sourcing /home/betta/.vim/bundle/syntastic/autoload/syntastic/util.vim
059.730  000.126  000.126: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/autoloclist.vim
059.883  000.117  000.117: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/balloons.vim
060.273  000.368  000.368: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/checker.vim
060.446  000.150  000.150: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/cursor.vim
060.582  000.116  000.116: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/highlighting.vim
061.049  000.439  000.439: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/loclist.vim
061.272  000.145  000.145: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/modemap.vim
061.434  000.129  000.129: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/notifiers.vim
061.940  000.481  000.481: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/registry.vim
062.125  000.150  000.150: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/signs.vim
064.571  014.191  011.160: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic.vim
065.540  000.170  000.170: sourcing /home/betta/.vim/bundle/ultisnips/autoload/UltiSnips/map_keys.vim
065.711  000.939  000.769: sourcing /home/betta/.vim/bundle/ultisnips/plugin/UltiSnips.vim
065.932  000.094  000.094: sourcing /home/betta/.vim/bundle/vim-snippets/plugin/vimsnippets.vim
067.042  001.013  001.013: sourcing /home/betta/.vim/bundle/YouCompleteMe/plugin/youcompleteme.vim
068.566  000.571  000.571: sourcing /home/betta/.vim/bundle/delimitMate/autoload/delimitMate.vim
072.452  005.267  004.696: sourcing /home/betta/.vim/bundle/delimitMate/plugin/delimitMate.vim
072.839  000.139  000.139: sourcing /home/betta/.vim/bundle/rust.vim/plugin/cargo.vim
072.931  000.073  000.073: sourcing /home/betta/.vim/bundle/rust.vim/plugin/rust.vim
080.435  007.397  007.397: sourcing /home/betta/.vim/bundle/nerdcommenter/plugin/NERD_commenter.vim
081.488  000.872  000.872: sourcing /home/betta/.vim/bundle/vim-surround/plugin/surround.vim
081.832  000.168  000.168: sourcing /home/betta/.vim/bundle/vim-trailing-whitespace/plugin/trailing-whitespace.vim
082.392  000.401  000.401: sourcing /home/betta/.vim/bundle/vim-easy-align/plugin/easy_align.vim
094.288  011.758  011.758: sourcing /home/betta/.vim/bundle/vim-easymotion/plugin/EasyMotion.vim
096.031  001.517  001.517: sourcing /home/betta/.vim/bundle/quick-scope/plugin/quick_scope.vim
097.342  001.134  001.134: sourcing /home/betta/.vim/bundle/matchit.zip/plugin/matchit.vim
098.088  000.229  000.229: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/utils.vim
100.327  002.809  002.580: sourcing /home/betta/.vim/bundle/vim-signature/plugin/signature.vim
101.192  000.395  000.395: sourcing /home/betta/.vim/bundle/vim-expand-region/autoload/expand_region.vim
101.400  000.868  000.473: sourcing /home/betta/.vim/bundle/vim-expand-region/plugin/expand_region.vim
101.932  000.428  000.428: sourcing /home/betta/.vim/bundle/vim-multiple-cursors/plugin/multiple_cursors.vim
102.826  000.255  000.255: sourcing /home/betta/.vim/bundle/ctrlp.vim/autoload/ctrlp/mrufiles.vim
103.342  001.291  001.036: sourcing /home/betta/.vim/bundle/ctrlp.vim/plugin/ctrlp.vim
103.549  000.054  000.054: sourcing /home/betta/.vim/bundle/ctrlp-funky/plugin/funky.vim
104.273  000.300  000.300: sourcing /home/betta/.vim/bundle/ctrlsf.vim/autoload/ctrlsf/backend.vim
105.282  001.667  001.367: sourcing /home/betta/.vim/bundle/ctrlsf.vim/plugin/ctrlsf.vim
105.564  000.170  000.170: sourcing /home/betta/.vim/bundle/vim-quickrun/plugin/quickrun.vim
106.309  000.637  000.637: sourcing /home/betta/.vim/bundle/vim-fugitive/plugin/fugitive.vim
107.107  000.230  000.230: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter/utility.vim
107.832  000.142  000.142: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter/highlight.vim
109.408  003.008  002.636: sourcing /home/betta/.vim/bundle/vim-gitgutter/plugin/gitgutter.vim
109.622  000.092  000.092: sourcing /home/betta/.vim/bundle/gundo.vim/plugin/gundo.vim
110.611  000.177  000.177: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/init.vim
111.426  000.144  000.144: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/parts.vim
113.210  000.198  000.198: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/util.vim
113.262  003.551  003.032: sourcing /home/betta/.vim/bundle/vim-airline/plugin/airline.vim
113.402  000.037  000.037: sourcing /home/betta/.vim/bundle/vim-airline-themes/plugin/airline-themes.vim
113.572  000.090  000.090: sourcing /home/betta/.vim/bundle/rainbow_parentheses.vim/plugin/rainbow_parentheses.vim
114.548  000.161  000.161: sourcing /home/betta/.vim/bundle/nerdtree/autoload/nerdtree.vim
116.739  001.220  001.220: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/path.vim
117.568  000.444  000.444: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/menu_controller.vim
118.113  000.247  000.247: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/menu_item.vim
118.649  000.269  000.269: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/key_map.vim
119.630  000.712  000.712: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/bookmark.vim
120.618  000.680  000.680: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/tree_file_node.vim
121.724  000.810  000.810: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/tree_dir_node.vim
122.230  000.223  000.223: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/opener.vim
122.795  000.252  000.252: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/creator.vim
123.119  000.052  000.052: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/flag_set.vim
123.559  000.185  000.185: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/nerdtree.vim
124.173  000.351  000.351: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/ui.vim
124.480  000.050  000.050: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/event.vim
124.805  000.052  000.052: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/notifier.vim
125.697  000.512  000.512: sourcing /home/betta/.vim/bundle/nerdtree/autoload/nerdtree/ui_glue.vim
129.250  000.144  000.144: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/exec_menuitem.vim
130.027  000.744  000.744: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/fs_menu.vim
130.138  000.067  000.067: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/vcs.vim
131.378  001.121  001.121: sourcing /home/betta/.vim/bundle/vim-nerdtree-tabs/nerdtree_plugin/vim-nerdtree-tabs.vim
131.860  018.185  009.889: sourcing /home/betta/.vim/bundle/nerdtree/plugin/NERD_tree.vim
140.244  007.485  007.485: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/context.vim
141.176  000.517  000.517: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/modes.vim
141.263  008.858  000.856: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/init.vim
142.122  000.185  000.185: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys.vim
142.748  000.175  000.175: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/roots.vim
143.242  000.174  000.174: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/bookmarks.vim
147.277  000.470  000.470: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/common.vim
148.841  000.519  000.519: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/help.vim
149.544  000.168  000.168: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/help.vim
151.104  000.078  000.078: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/nop.vim
152.539  000.083  000.083: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/search.vim
154.094  000.316  000.316: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/buffer.vim
156.562  000.174  000.174: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/file.vim
158.636  000.179  000.179: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/tab.vim
159.913  000.139  000.139: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/workspace.vim
160.749  000.095  000.095: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/bookmark.vim
161.282  029.304  017.691: sourcing /home/betta/.vim/bundle/vim-ctrlspace/plugin/ctrlspace.vim
162.736  001.326  001.326: sourcing /home/betta/.vim/bundle/tagbar/plugin/tagbar.vim
163.973  000.666  000.666: sourcing /home/betta/.vim/bundle/vim-textobj-user/autoload/textobj/user.vim
165.934  003.006  002.340: sourcing /home/betta/.vim/bundle/vim-textobj-line/plugin/textobj/line.vim
167.921  001.832  001.832: sourcing /home/betta/.vim/bundle/vim-textobj-entire/plugin/textobj/entire.vim
171.375  003.317  003.317: sourcing /home/betta/.vim/bundle/vim-textobj-indent/plugin/textobj/indent.vim
171.722  000.215  000.215: sourcing /home/betta/.vim/bundle/vim-tmux-navigator/plugin/tmux_navigator.vim
217.221  045.357  045.357: sourcing /home/betta/.vim/bundle/vim-isort/plugin/python_vimisort.vim
218.258  000.253  000.253: sourcing /home/betta/.vim/bundle/vim-indent-guides/autoload/indent_guides.vim
219.080  001.634  001.381: sourcing /home/betta/.vim/bundle/vim-indent-guides/plugin/indent_guides.vim
219.636  000.177  000.177: sourcing /usr/share/vim/vim80/plugin/getscriptPlugin.vim
220.205  000.526  000.526: sourcing /usr/share/vim/vim80/plugin/gzip.vim
220.570  000.324  000.324: sourcing /usr/share/vim/vim80/plugin/logiPat.vim
220.617  000.021  000.021: sourcing /usr/share/vim/vim80/plugin/manpager.vim
220.876  000.234  000.234: sourcing /usr/share/vim/vim80/plugin/matchparen.vim
222.003  001.068  001.068: sourcing /usr/share/vim/vim80/plugin/netrwPlugin.vim
222.093  000.028  000.028: sourcing /usr/share/vim/vim80/plugin/rrhelper.vim
222.181  000.056  000.056: sourcing /usr/share/vim/vim80/plugin/spellfile.vim
222.464  000.248  000.248: sourcing /usr/share/vim/vim80/plugin/tarPlugin.vim
222.618  000.116  000.116: sourcing /usr/share/vim/vim80/plugin/tohtml.vim
222.920  000.272  000.272: sourcing /usr/share/vim/vim80/plugin/vimballPlugin.vim
223.234  000.245  000.245: sourcing /usr/share/vim/vim80/plugin/zipPlugin.vim
223.248  006.559: loading plugins
223.339  000.091: loading packages
223.743  000.240  000.240: sourcing /home/betta/.vim/bundle/ultisnips/after/plugin/UltiSnips_after.vim
223.930  000.073  000.073: sourcing /home/betta/.vim/bundle/vim-signature/after/plugin/signature.vim
224.570  000.236  000.236: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline.vim
224.609  000.594  000.358: sourcing /home/betta/.vim/bundle/ctrlsf.vim/after/plugin/ctrlsf.vim
224.665  000.419: loading after plugins
224.684  000.019: inits 3
224.693  000.009: reading viminfo
224.749  000.056: setting raw mode
224.757  000.008: start termcap
224.846  000.089: clearing screen
226.036  000.263  000.263: sourcing /home/betta/.vim/bundle/syntastic/autoload/syntastic/log.vim
226.787  000.322  000.322: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions.vim
227.225  000.081  000.081: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/quickfix.vim
227.639  000.055  000.055: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/netrw.vim
228.015  000.059  000.059: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/term.vim
228.435  000.086  000.086: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/ctrlp.vim
228.834  000.049  000.049: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/ctrlspace.vim
229.278  000.109  000.109: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/hunks.vim
229.758  000.086  000.086: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/tagbar.vim
230.401  000.261  000.261: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/branch.vim
230.938  000.075  000.075: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/fugitiveline.vim
231.652  000.129  000.129: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/syntastic.vim
232.414  000.241  000.241: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/whitespace.vim
233.513  000.126  000.126: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/po.vim
234.182  000.191  000.191: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/wordcount.vim
234.878  000.071  000.071: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/keymap.vim
239.526  000.147  000.147: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/section.vim
240.213  000.295  000.295: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/highlighter.vim
247.867  000.093  000.093: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/themes.vim
248.242  000.807  000.714: sourcing /home/betta/.vim/bundle/vim-airline-themes/autoload/airline/themes/molokai.vim
273.876  000.262  000.262: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/builder.vim
274.719  000.131  000.131: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/default.vim
334.382  105.690: opening buffers
339.697  000.324  000.324: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/sign.vim
340.472  000.337  000.337: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/mark.vim
345.029  000.231  000.231: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter.vim
346.173  000.496  000.496: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/buffers.vim
346.842  000.138  000.138: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/jumps.vim
347.006  011.098: BufEnter autocommands
347.010  000.004: editing files in windows
347.895  000.345  000.345: sourcing /home/betta/.vim/bundle/rainbow_parentheses.vim/autoload/rainbow_parentheses.vim
350.470  001.168  001.168: sourcing /home/betta/.vim/bundle/YouCompleteMe/autoload/youcompleteme.vim
510.058  009.419  009.419: sourcing /home/betta/.vim/bundle/vim-fugitive/autoload/fugitive.vim
512.782  154.840: VimEnter autocommands
512.786  000.004: before starting main loop
513.148  000.362: first screen update
513.151  000.003: --- VIM STARTED ---


times in msec
 clock   self+sourced   self:  sourced script
 clock   elapsed:              other lines

000.005  000.005: --- VIM STARTING ---
000.106  000.101: Allocated generic buffers
000.217  000.111: locale set
000.224  000.007: window checked
000.616  000.392: inits 1
000.647  000.031: parsing arguments
000.648  000.001: expanding arguments
000.664  000.016: shell init
000.947  000.283: Termcap init
000.965  000.018: inits 2
001.097  000.132: init highlight
001.470  000.277  000.277: sourcing /usr/share/vim/vim80/debian.vim
002.887  001.232  001.232: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
003.075  001.487  000.255: sourcing /usr/share/vim/vim80/syntax/synload.vim
011.813  008.670  008.670: sourcing /usr/share/vim/vim80/filetype.vim
011.910  010.386  000.229: sourcing /usr/share/vim/vim80/syntax/syntax.vim
011.972  010.844  000.181: sourcing $VIM/vimrc
012.770  000.530  000.530: sourcing /usr/share/vim/vim80/syntax/nosyntax.vim
013.431  000.420  000.420: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
013.669  000.797  000.377: sourcing /usr/share/vim/vim80/syntax/synload.vim
013.796  001.643  000.316: sourcing /usr/share/vim/vim80/syntax/syntax.vim
015.434  001.214  001.214: sourcing /usr/share/vim/vim80/ftoff.vim
019.240  003.059  003.059: sourcing /home/betta/.vim/autoload/plug.vim
031.594  000.118  000.118: sourcing /home/betta/.vim/bundle/vim-markdown/ftdetect/markdown.vim
048.461  000.067  000.067: sourcing /home/betta/.vim/bundle/ultisnips/ftdetect/snippets.vim
048.707  000.076  000.076: sourcing /home/betta/.vim/bundle/vim-openresty/ftdetect/nginx.vim
048.888  000.089  000.089: sourcing /home/betta/.vim/bundle/rust.vim/ftdetect/rust.vim
049.284  000.029  000.029: sourcing /home/betta/.vim/bundle/vim-fugitive/ftdetect/fugitive.vim
049.844  000.102  000.102: sourcing /home/betta/.vim/bundle/Vim-Jinja2-Syntax/ftdetect/jinja.vim
050.262  000.273  000.273: sourcing /home/betta/.vim/bundle/vim-javascript/ftdetect/javascript.vim
050.661  014.706  014.070: sourcing /usr/share/vim/vim80/filetype.vim
051.550  000.102  000.102: sourcing /usr/share/vim/vim80/ftplugin.vim
052.397  000.089  000.089: sourcing /usr/share/vim/vim80/indent.vim
054.304  040.408  021.120: sourcing /home/betta/.vimrc.bundles
054.976  000.037  000.037: sourcing /usr/share/vim/vim80/filetype.vim
055.669  000.032  000.032: sourcing /usr/share/vim/vim80/ftplugin.vim
056.395  000.028  000.028: sourcing /usr/share/vim/vim80/indent.vim
057.192  000.034  000.034: sourcing /usr/share/vim/vim80/filetype.vim
058.025  000.031  000.031: sourcing /usr/share/vim/vim80/filetype.vim
058.688  000.027  000.027: sourcing /usr/share/vim/vim80/indent.vim
059.370  000.030  000.030: sourcing /usr/share/vim/vim80/filetype.vim
060.063  000.028  000.028: sourcing /usr/share/vim/vim80/ftplugin.vim
060.754  000.028  000.028: sourcing /usr/share/vim/vim80/filetype.vim
061.437  000.026  000.026: sourcing /usr/share/vim/vim80/ftplugin.vim
062.219  000.028  000.028: sourcing /usr/share/vim/vim80/indent.vim
065.885  000.450  000.450: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
067.934  000.426  000.426: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
069.557  000.374  000.374: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
070.838  000.374  000.374: sourcing /usr/share/vim/vim80/syntax/syncolor.vim
072.106  005.650  004.476: sourcing /home/betta/.vim/bundle/molokai/colors/molokai.vim
072.216  060.166  011.686: sourcing $HOME/.vimrc
072.223  000.116: sourcing vimrc file(s)
073.155  000.049  000.049: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/autoloclist.vim
073.222  000.028  000.028: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/balloons.vim
073.285  000.029  000.029: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/checker.vim
073.351  000.033  000.033: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/cursor.vim
073.414  000.029  000.029: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/highlighting.vim
073.475  000.028  000.028: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/loclist.vim
073.604  000.030  000.030: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/modemap.vim
073.676  000.037  000.037: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/notifiers.vim
073.776  000.033  000.033: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/registry.vim
073.842  000.029  000.029: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/signs.vim
075.245  001.028  001.028: sourcing /home/betta/.vim/bundle/syntastic/autoload/syntastic/util.vim
084.364  000.195  000.195: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/autoloclist.vim
085.598  001.185  001.185: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/balloons.vim
086.234  000.598  000.598: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/checker.vim
086.480  000.195  000.195: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/cursor.vim
086.656  000.143  000.143: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/highlighting.vim
087.256  000.567  000.567: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/loclist.vim
087.470  000.168  000.168: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/modemap.vim
087.684  000.175  000.175: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/notifiers.vim
088.353  000.635  000.635: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/registry.vim
088.636  000.232  000.232: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic/signs.vim
091.899  018.019  013.898: sourcing /home/betta/.vim/bundle/syntastic/plugin/syntastic.vim
092.905  000.235  000.235: sourcing /home/betta/.vim/bundle/ultisnips/autoload/UltiSnips/map_keys.vim
093.146  000.993  000.758: sourcing /home/betta/.vim/bundle/ultisnips/plugin/UltiSnips.vim
093.424  000.114  000.114: sourcing /home/betta/.vim/bundle/vim-snippets/plugin/vimsnippets.vim
094.761  001.207  001.207: sourcing /home/betta/.vim/bundle/YouCompleteMe/plugin/youcompleteme.vim
096.765  000.844  000.844: sourcing /home/betta/.vim/bundle/delimitMate/autoload/delimitMate.vim
103.051  008.138  007.294: sourcing /home/betta/.vim/bundle/delimitMate/plugin/delimitMate.vim
103.595  000.226  000.226: sourcing /home/betta/.vim/bundle/rust.vim/plugin/cargo.vim
103.729  000.093  000.093: sourcing /home/betta/.vim/bundle/rust.vim/plugin/rust.vim
114.428  010.547  010.547: sourcing /home/betta/.vim/bundle/nerdcommenter/plugin/NERD_commenter.vim
115.969  001.287  001.287: sourcing /home/betta/.vim/bundle/vim-surround/plugin/surround.vim
116.363  000.191  000.191: sourcing /home/betta/.vim/bundle/vim-trailing-whitespace/plugin/trailing-whitespace.vim
117.030  000.475  000.475: sourcing /home/betta/.vim/bundle/vim-easy-align/plugin/easy_align.vim
132.050  014.848  014.848: sourcing /home/betta/.vim/bundle/vim-easymotion/plugin/EasyMotion.vim
134.141  001.812  001.812: sourcing /home/betta/.vim/bundle/quick-scope/plugin/quick_scope.vim
135.760  001.423  001.423: sourcing /home/betta/.vim/bundle/matchit.zip/plugin/matchit.vim
136.602  000.260  000.260: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/utils.vim
139.565  003.604  003.344: sourcing /home/betta/.vim/bundle/vim-signature/plugin/signature.vim
140.784  000.548  000.548: sourcing /home/betta/.vim/bundle/vim-expand-region/autoload/expand_region.vim
141.130  001.322  000.774: sourcing /home/betta/.vim/bundle/vim-expand-region/plugin/expand_region.vim
141.954  000.630  000.630: sourcing /home/betta/.vim/bundle/vim-multiple-cursors/plugin/multiple_cursors.vim
143.233  000.434  000.434: sourcing /home/betta/.vim/bundle/ctrlp.vim/autoload/ctrlp/mrufiles.vim
143.913  001.816  001.382: sourcing /home/betta/.vim/bundle/ctrlp.vim/plugin/ctrlp.vim
144.234  000.094  000.094: sourcing /home/betta/.vim/bundle/ctrlp-funky/plugin/funky.vim
145.232  000.383  000.383: sourcing /home/betta/.vim/bundle/ctrlsf.vim/autoload/ctrlsf/backend.vim
146.970  002.610  002.227: sourcing /home/betta/.vim/bundle/ctrlsf.vim/plugin/ctrlsf.vim
147.494  000.308  000.308: sourcing /home/betta/.vim/bundle/vim-quickrun/plugin/quickrun.vim
148.633  000.944  000.944: sourcing /home/betta/.vim/bundle/vim-fugitive/plugin/fugitive.vim
149.971  000.338  000.338: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter/utility.vim
151.274  000.267  000.267: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter/highlight.vim
154.022  005.213  004.608: sourcing /home/betta/.vim/bundle/vim-gitgutter/plugin/gitgutter.vim
154.370  000.146  000.146: sourcing /home/betta/.vim/bundle/gundo.vim/plugin/gundo.vim
155.594  000.202  000.202: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/init.vim
156.892  000.227  000.227: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/parts.vim
160.037  000.275  000.275: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/util.vim
160.127  005.612  004.908: sourcing /home/betta/.vim/bundle/vim-airline/plugin/airline.vim
160.387  000.072  000.072: sourcing /home/betta/.vim/bundle/vim-airline-themes/plugin/airline-themes.vim
160.710  000.164  000.164: sourcing /home/betta/.vim/bundle/rainbow_parentheses.vim/plugin/rainbow_parentheses.vim
162.607  000.303  000.303: sourcing /home/betta/.vim/bundle/nerdtree/autoload/nerdtree.vim
167.101  002.376  002.376: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/path.vim
168.498  000.786  000.786: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/menu_controller.vim
169.557  000.541  000.541: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/menu_item.vim
170.560  000.568  000.568: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/key_map.vim
172.546  001.515  001.515: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/bookmark.vim
174.550  001.497  001.497: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/tree_file_node.vim
176.481  001.346  001.346: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/tree_dir_node.vim
177.290  000.361  000.361: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/opener.vim
178.199  000.407  000.407: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/creator.vim
178.885  000.123  000.123: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/flag_set.vim
179.691  000.297  000.297: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/nerdtree.vim
180.787  000.605  000.605: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/ui.vim
181.303  000.079  000.079: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/event.vim
181.849  000.095  000.095: sourcing /home/betta/.vim/bundle/nerdtree/lib/nerdtree/notifier.vim
183.223  000.865  000.865: sourcing /home/betta/.vim/bundle/nerdtree/autoload/nerdtree/ui_glue.vim
189.864  000.284  000.284: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/exec_menuitem.vim
191.370  001.435  001.435: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/fs_menu.vim
191.573  000.126  000.126: sourcing /home/betta/.vim/bundle/nerdtree/nerdtree_plugin/vcs.vim
193.593  001.819  001.819: sourcing /home/betta/.vim/bundle/vim-nerdtree-tabs/nerdtree_plugin/vim-nerdtree-tabs.vim
194.397  033.491  018.063: sourcing /home/betta/.vim/bundle/nerdtree/plugin/NERD_tree.vim
202.271  006.247  006.247: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/context.vim
203.625  000.711  000.711: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/modes.vim
203.757  008.370  001.412: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/init.vim
204.955  000.281  000.281: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys.vim
205.927  000.260  000.260: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/roots.vim
206.814  000.353  000.353: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/bookmarks.vim
213.574  000.570  000.570: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/common.vim
216.315  000.881  000.881: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/help.vim
217.403  000.258  000.258: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/help.vim
220.183  000.121  000.121: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/nop.vim
222.775  000.175  000.175: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/search.vim
225.182  000.556  000.556: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/buffer.vim
229.615  000.299  000.299: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/file.vim
232.784  000.259  000.259: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/tab.vim
235.024  000.234  000.234: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/workspace.vim
236.454  000.146  000.146: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/keys/bookmark.vim
237.224  042.607  029.844: sourcing /home/betta/.vim/bundle/vim-ctrlspace/plugin/ctrlspace.vim
239.422  002.025  002.025: sourcing /home/betta/.vim/bundle/tagbar/plugin/tagbar.vim
241.236  000.914  000.914: sourcing /home/betta/.vim/bundle/vim-textobj-user/autoload/textobj/user.vim
244.343  004.647  003.733: sourcing /home/betta/.vim/bundle/vim-textobj-line/plugin/textobj/line.vim
247.514  002.932  002.932: sourcing /home/betta/.vim/bundle/vim-textobj-entire/plugin/textobj/entire.vim
253.171  005.442  005.442: sourcing /home/betta/.vim/bundle/vim-textobj-indent/plugin/textobj/indent.vim
253.686  000.314  000.314: sourcing /home/betta/.vim/bundle/vim-tmux-navigator/plugin/tmux_navigator.vim
319.799  065.906  065.906: sourcing /home/betta/.vim/bundle/vim-isort/plugin/python_vimisort.vim
321.454  000.404  000.404: sourcing /home/betta/.vim/bundle/vim-indent-guides/autoload/indent_guides.vim
322.569  002.457  002.053: sourcing /home/betta/.vim/bundle/vim-indent-guides/plugin/indent_guides.vim
323.299  000.181  000.181: sourcing /usr/share/vim/vim80/plugin/getscriptPlugin.vim
323.885  000.547  000.547: sourcing /usr/share/vim/vim80/plugin/gzip.vim
324.457  000.525  000.525: sourcing /usr/share/vim/vim80/plugin/logiPat.vim
324.526  000.030  000.030: sourcing /usr/share/vim/vim80/plugin/manpager.vim
324.933  000.373  000.373: sourcing /usr/share/vim/vim80/plugin/matchparen.vim
326.553  001.542  001.542: sourcing /usr/share/vim/vim80/plugin/netrwPlugin.vim
326.671  000.038  000.038: sourcing /usr/share/vim/vim80/plugin/rrhelper.vim
326.828  000.074  000.074: sourcing /usr/share/vim/vim80/plugin/spellfile.vim
327.238  000.356  000.356: sourcing /usr/share/vim/vim80/plugin/tarPlugin.vim
327.491  000.191  000.191: sourcing /usr/share/vim/vim80/plugin/tohtml.vim
327.933  000.393  000.393: sourcing /usr/share/vim/vim80/plugin/vimballPlugin.vim
328.381  000.354  000.354: sourcing /usr/share/vim/vim80/plugin/zipPlugin.vim
328.402  009.521: loading plugins
328.527  000.125: loading packages
329.115  000.356  000.356: sourcing /home/betta/.vim/bundle/ultisnips/after/plugin/UltiSnips_after.vim
329.402  000.108  000.108: sourcing /home/betta/.vim/bundle/vim-signature/after/plugin/signature.vim
330.416  000.390  000.390: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline.vim
330.478  000.941  000.551: sourcing /home/betta/.vim/bundle/ctrlsf.vim/after/plugin/ctrlsf.vim
330.575  000.643: loading after plugins
330.603  000.028: inits 3
330.614  000.011: reading viminfo
330.736  000.122: setting raw mode
330.750  000.014: start termcap
330.834  000.084: clearing screen
332.413  000.385  000.385: sourcing /home/betta/.vim/bundle/syntastic/autoload/syntastic/log.vim
333.575  000.539  000.539: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions.vim
334.334  000.120  000.120: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/quickfix.vim
335.118  000.094  000.094: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/netrw.vim
335.753  000.089  000.089: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/term.vim
336.429  000.138  000.138: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/ctrlp.vim
337.158  000.081  000.081: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/ctrlspace.vim
337.857  000.163  000.163: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/hunks.vim
338.728  000.171  000.171: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/tagbar.vim
339.840  000.433  000.433: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/branch.vim
340.605  000.109  000.109: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/fugitiveline.vim
341.344  000.139  000.139: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/syntastic.vim
342.299  000.263  000.263: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/whitespace.vim
343.454  000.128  000.128: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/po.vim
344.156  000.187  000.187: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/wordcount.vim
345.007  000.082  000.082: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/keymap.vim
352.564  000.199  000.199: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/section.vim
353.731  000.468  000.468: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/highlighter.vim
366.378  000.149  000.149: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/themes.vim
367.071  001.361  001.212: sourcing /home/betta/.vim/bundle/vim-airline-themes/autoload/airline/themes/molokai.vim
408.547  000.342  000.342: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/builder.vim
409.918  000.209  000.209: sourcing /home/betta/.vim/bundle/vim-airline/autoload/airline/extensions/default.vim
498.982  162.448: opening buffers
504.148  000.327  000.327: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/sign.vim
504.877  000.329  000.329: sourcing /home/betta/.vim/bundle/vim-signature/autoload/signature/mark.vim
509.123  000.189  000.189: sourcing /home/betta/.vim/bundle/vim-gitgutter/autoload/gitgutter.vim
510.120  000.436  000.436: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/buffers.vim
510.615  000.102  000.102: sourcing /home/betta/.vim/bundle/vim-ctrlspace/autoload/ctrlspace/jumps.vim
510.780  010.415: BufEnter autocommands
510.783  000.003: editing files in windows
511.465  000.266  000.266: sourcing /home/betta/.vim/bundle/rainbow_parentheses.vim/autoload/rainbow_parentheses.vim
513.334  000.820  000.820: sourcing /home/betta/.vim/bundle/YouCompleteMe/autoload/youcompleteme.vim
649.552  005.737  005.737: sourcing /home/betta/.vim/bundle/vim-fugitive/autoload/fugitive.vim
651.300  133.694: VimEnter autocommands
651.304  000.004: before starting main loop
651.495  000.191: first screen update
651.497  000.002: --- VIM STARTED ---
