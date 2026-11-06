/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

// Served from 127.0.0.1. Reports "done" once its own page request and its frame's
// have both been answered.
const TOP: &str = "<!doctype html><html><head><title>start</title></head><body><script>\
    let pending = 2;\
    const finish = () => { if (--pending === 0) document.title = 'done'; };\
    onmessage = e => { if (e.data === 'ok') finish(); else document.title = 'rej:frame:' + e.data; };\
    EmbedderRequest.send(new Uint8Array([1])).then(finish, e => { document.title = 'rej:top:' + e.name; });\
    </script><iframe src=\"http://localhost:PORT/frame\"></iframe></body></html>";

// Served from localhost, a different origin from the page that embeds it.
const FRAME: &str = "<!doctype html><script>\
    EmbedderRequest.send(new Uint8Array([2])).then(\
    () => parent.postMessage('ok', '*'), e => parent.postMessage(e.name, '*'));\
    </script>";

fn serve(mut stream: TcpStream, port: u16) {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let path = request.split_whitespace().nth(1).unwrap_or("");
    let (status, body) = match path {
        "/top" => ("200 OK", TOP.replace("PORT", &port.to_string())),
        "/frame" => ("200 OK", FRAME.to_owned()),
        _ => ("404 Not Found", String::new()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
}

fn listen(listener: TcpListener, port: u16) {
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || serve(stream, port));
        }
    });
}

#[test]
fn c_control_origin_tests() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("could not bind a loopback port");
    let port = listener.local_addr().unwrap().port();
    listen(listener, port);
    // "localhost" may resolve to the IPv6 loopback first; serve it there too when the
    // host has one. Neither address leaves the machine.
    if let Ok(listener) = TcpListener::bind(("::1", port)) {
        listen(listener, port);
    }

    servo_capi_tests::c_control_origin_tests(port);
}
