use super::socket_server::Rooms;
use std::net::Ipv4Addr;
use tokio::io;
use warp::Filter;

pub async fn start(port: u16, rooms: &'static Rooms) -> io::Result<()> {
    println!("Serving on port: {}", port);
    let room_route =
        warp::path("rooms").map(move || format!("{:?}", rooms.list()));
    let run = warp::path!("run" / String / String).and_then(
        move |name: String, s: String| async move {
            let (name, s) = dbg!((name, s));
            let args = crate::arg_split::quoted_parse(&s)
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if rooms.run_cmd(&name, args).await {
                Ok(warp::reply())
            } else {
                Err(warp::reject::not_found())
            }
        },
    );
    let get = warp::path!("get" / String / String).and_then(
        move |name: String, s: String| async move {
            let args = crate::arg_split::quoted_parse(&s)
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if let Some(r) = rooms.get_cmd(&name, args).await {
                Ok(r)
            } else {
                Err(warp::reject::not_found())
            }
        },
    );
    warp::serve(room_route.or(run).or(get))
        .run((Ipv4Addr::UNSPECIFIED, port))
        .await;
    Ok(())
}
