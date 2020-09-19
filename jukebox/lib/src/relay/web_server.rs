use crate::{relay::rooms::Rooms, RoomName};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::{convert::Infallible, net::Ipv4Addr};
use tokio::io;
use warp::{
    http::StatusCode,
    reply::{with_status, WithStatus},
    Filter, Rejection,
};

type OkResult<T> = Result<T, Infallible>;

async fn get_cmd(
    rooms: &Rooms,
    name: RoomName,
    s: String,
) -> OkResult<WithStatus<String>> {
    let args = crate::arg_split::quoted_parse(&s)
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    if let Some(r) = rooms.get_cmd(&name, args).await {
        match r {
            Ok(ok) => Ok(with_status(ok, StatusCode::OK)),
            Err(err) => Ok(with_status(err, StatusCode::BAD_REQUEST)),
        }
    } else {
        Ok(with_status(
            "Jukebox not active".into(),
            StatusCode::NOT_FOUND,
        ))
    }
}

async fn run_cmd(
    rooms: &Rooms,
    name: RoomName,
    s: String,
) -> OkResult<WithStatus<&'static str>> {
    let args = crate::arg_split::quoted_parse(&s)
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    if rooms.run_cmd(&name, args).await {
        Ok(with_status("Done", StatusCode::OK))
    } else {
        Ok(with_status("Jukebox not active", StatusCode::NOT_FOUND))
    }
}

pub async fn start(port: u16, rooms: &'static Rooms) -> io::Result<()> {
    println!("Serving on port: {}", port);
    let room_route =
        warp::path("rooms").map(move || warp::reply::json(&rooms.list()));
    let run = warp::path!("run" / RoomName / String)
        .and_then(move |name, s| run_cmd(rooms, name, s));
    let get = warp::path!("get" / RoomName / String)
        .and_then(move |name, s| get_cmd(rooms, name, s));
    let run_body = warp::path!("run" / RoomName)
        .and(warp::body::json())
        .and_then(move |name, req: Req| run_cmd(rooms, name, req.cmd_line));
    let get_body = warp::path!("get" / RoomName)
        .and(warp::body::json())
        .and_then(move |name, req: Req| get_cmd(rooms, name, req.cmd_line));
    warp::serve(
        room_route
            .or(run)
            .or(get)
            .or(run_body)
            .or(get_body)
            .recover(handle_rejection),
    )
    .run((Ipv4Addr::UNSPECIFIED, port))
    .await;
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Req {
    cmd_line: String,
}

impl From<String> for Req {
    fn from(cmd_line: String) -> Self {
        Self { cmd_line }
    }
}

// This function receives a `Rejection` and tries to return a custom
// value, otherwise simply passes the rejection along.
async fn handle_rejection(
    err: Rejection,
) -> Result<WithStatus<&'static str>, Infallible> {
    let code;
    let message;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "NOT_FOUND";
    } else if let Some(e) =
        err.find::<warp::filters::body::BodyDeserializeError>()
    {
        // This error happens if the body could not be deserialized correctly
        // We can use the cause to analyze the error and customize the error message
        message = match e.source() {
            Some(cause) => {
                if cause.to_string().contains("denom") {
                    "FIELD_ERROR: denom"
                } else {
                    "BAD_REQUEST"
                }
            }
            None => "BAD_REQUEST",
        };
        code = StatusCode::BAD_REQUEST;
    } else if let Some(_) = err.find::<warp::reject::MethodNotAllowed>() {
        // We can handle a specific error, here METHOD_NOT_ALLOWED,
        // and render it however we want
        code = StatusCode::METHOD_NOT_ALLOWED;
        message = "METHOD_NOT_ALLOWED";
    } else {
        // We should have expected this... Just log and say its a 500
        eprintln!("unhandled rejection: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "UNHANDLED_REJECTION";
    }

    Ok(warp::reply::with_status(message, code))
}
