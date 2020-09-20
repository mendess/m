use crate::{
    relay::rooms::{CommandResult, Rooms},
    RoomName,
};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, error::Error, net::Ipv4Addr};
use tokio::io;
use tracing_subscriber::fmt::format::FmtSpan;
use warp::{
    http::Method,
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
            CommandResult::Success(Ok(ok)) => {
                Ok(with_status(ok, StatusCode::OK))
            }
            CommandResult::Success(Err(err)) => {
                Ok(with_status(err, StatusCode::BAD_REQUEST))
            }
            CommandResult::BoxOfflineWarning => {
                Ok(with_status("Jukebox offline".into(), StatusCode::OK))
            }
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
    match rooms.run_cmd(&name, args).await {
        Some(CommandResult::Success(_)) => {
            Ok(with_status("Done", StatusCode::OK))
        }
        Some(CommandResult::BoxOfflineWarning) => {
            Ok(with_status("Jukebox offline", StatusCode::OK))
        }
        None => Ok(with_status("Jukebox doesn't exist", StatusCode::NOT_FOUND)),
    }
}

pub async fn start(port: u16, rooms: &'static Rooms) -> io::Result<()> {
    println!("Serving on port: {}", port);

    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "tracing=info,warp=debug".to_owned());

    tracing_subscriber::fmt()
        // Use the filter we built above to determine which traces to record.
        .with_env_filter(filter)
        // Record an event when each span closes. This can be used to time our
        // routes' durations!
        .with_span_events(FmtSpan::CLOSE)
        .init();

    let room_route = warp::path("rooms")
        .map(move || warp::reply::json(&rooms.list()))
        .with(warp::trace::named("rooms"));

    let run = warp::path!("run" / RoomName / String)
        .and_then(move |name, s| run_cmd(rooms, name, s))
        .with(warp::trace::named("run"));

    let get = warp::path!("get" / RoomName / String)
        .and_then(move |name, s| get_cmd(rooms, name, s))
        .with(warp::trace::named("get"));

    let run_body = warp::path!("run" / RoomName)
        .and(warp::body::json())
        .and_then(move |name, req: Req| run_cmd(rooms, name, req.cmd_line))
        .with(warp::trace::named("run_body"));

    let get_body = warp::path!("get" / RoomName)
        .and(warp::body::json())
        .and_then(move |name, req: Req| get_cmd(rooms, name, req.cmd_line))
        .with(warp::trace::named("get_body"));

    let get_routes = warp::get().and(room_route.or(run).or(get));
    let post_routes = warp::post().and(run_body.or(get_body));

    warp::serve(
        post_routes
            .or(get_routes)
            .with(
                warp::cors()
                    .allow_any_origin()
                    .allow_methods(vec![Method::GET, Method::POST])
                    .allow_headers(vec!["content-type"])
                    .build(),
            )
            .with(warp::trace::request())
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
    let err = dbg!(err);
    let (code, message) = if err.is_not_found() {
        (StatusCode::NOT_FOUND, "NOT_FOUND")
    } else if let Some(e) =
        err.find::<warp::filters::body::BodyDeserializeError>()
    {
        // This error happens if the body could not be deserialized correctly
        // We can use the cause to analyze the error and customize the error message
        let message = match e.source() {
            Some(cause) => {
                if cause.to_string().contains("denom") {
                    "FIELD_ERROR: denom"
                } else {
                    "BAD_REQUEST"
                }
            }
            None => "BAD_REQUEST",
        };
        (StatusCode::BAD_REQUEST, message)
    } else if let Some(_) = err.find::<warp::reject::MethodNotAllowed>() {
        // We can handle a specific error, here METHOD_NOT_ALLOWED,
        // and render it however we want
        (StatusCode::METHOD_NOT_ALLOWED, "METHOD_NOT_ALLOWED")
    } else {
        // We should have expected this... Just log and say its a 500
        eprintln!("unhandled rejection: {:?}", err);
        (StatusCode::INTERNAL_SERVER_ERROR, "UNHANDLED_REJECTION")
    };

    Ok(warp::reply::with_status(message, code))
}
