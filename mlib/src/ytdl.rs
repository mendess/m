pub mod util;

use std::process::ExitStatus;

use tokio::{io, process::Command};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("status {status_code}, because: {stderr}")]
    YtdlFailed {
        status_code: ExitStatus,
        stderr: String,
    },
}

mod sealed {
    pub trait Sealed {}
    impl<T> Sealed for super::Title<T> {}
    impl<T> Sealed for super::Duration<T> {}
    impl Sealed for super::VidId {}
    impl<T> Sealed for super::TitleRequest<T> {}
    impl<T> Sealed for super::DurationRequest<T> {}
    impl Sealed for super::LinkRequest<'_> {}
}

pub struct TitleRequest<T>(T);
pub struct Title<T> {
    title: String,
    tail: T,
}

pub struct DurationRequest<T>(T);
pub struct Duration<T> {
    duration: std::time::Duration,
    tail: T,
}

pub struct LinkRequest<'a>(&'a str);
pub struct VidId(String);

pub struct YtdlBuilder<T>(T);

impl<'l> YtdlBuilder<LinkRequest<'l>> {
    pub fn new(link: &'l str) -> Self {
        Self(LinkRequest(link))
    }
}

pub struct Ytdl<T>(T);

impl<T> YtdlBuilder<T> {
    pub fn get_title(self) -> YtdlBuilder<TitleRequest<T>> {
        YtdlBuilder(TitleRequest(self.0))
    }

    pub fn get_duration(self) -> YtdlBuilder<DurationRequest<T>> {
        YtdlBuilder(DurationRequest(self.0))
    }
}

impl<'l, Y, T: YtdlParam<'l> + IntoResponse<Output = Y>> YtdlBuilder<T> {
    pub async fn request(self) -> Result<Ytdl<Y>, Error> {
        let mut v = Vec::new();
        let link = self.0.link();
        v.push(link);
        T::collect(&mut v);
        let output = Command::new("youtube-dl").args(v).output().await?;
        if output.status.success() {
            let string = String::from_utf8_lossy(&output.stdout);
            let mut lines = string
                .split('\n')
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>();
            Ok(Ytdl(T::response(&mut lines)))
        } else {
            Err(Error::YtdlFailed {
                status_code: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

impl Ytdl<Title<Duration<VidId>>> {
    pub fn title(self) -> String {
        self.0.title
    }
}

impl Ytdl<Duration<Title<VidId>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }
}

impl Ytdl<Title<VidId>> {
    pub fn title(self) -> String {
        self.0.title
    }
}

impl Ytdl<Duration<Title<VidId>>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }
}

impl Ytdl<Title<Duration<VidId>>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }
}

impl Ytdl<Duration<VidId>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }
}

impl<R: Response> Ytdl<R> {
    pub fn id(&self) -> &str {
        self.0.id()
    }
}

pub trait YtdlParam<'l>: sealed::Sealed {
    fn collect(buf: &mut Vec<&str>);
    fn link(&self) -> &'l str;
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for TitleRequest<T> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-title");
        T::collect(buf);
    }
    fn link(&self) -> &'l str {
        self.0.link()
    }
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for DurationRequest<T> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-duration");
        T::collect(buf);
    }
    fn link(&self) -> &'l str {
        self.0.link()
    }
}

impl<'l> YtdlParam<'l> for LinkRequest<'l> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-id")
    }
    fn link(&self) -> &'l str {
        self.0
    }
}

pub trait IntoResponse: sealed::Sealed {
    type Output;
    fn response(buf: &mut Vec<&str>) -> Self::Output;
}

impl<Y, T: IntoResponse<Output = Y>> IntoResponse for TitleRequest<T> {
    type Output = Title<Y>;
    fn response(buf: &mut Vec<&str>) -> Self::Output {
        Self::Output {
            title: buf.swap_remove(0).to_string(),
            tail: T::response(buf),
        }
    }
}

impl<Y, T: IntoResponse<Output = Y>> IntoResponse for DurationRequest<T> {
    type Output = Duration<Y>;

    fn response(buf: &mut Vec<&str>) -> Self::Output {
        use std::time::Duration;

        const CTORS: &[fn(u64) -> Duration] = &[
            Duration::from_secs,
            |m| Duration::from_secs(m * 60),
            |h| Duration::from_secs(h * 60 * 60),
        ];

        let dur = buf.pop().unwrap();
        let total = dur
            .split(':')
            .rev()
            .map_while(|n| n.parse::<u64>().ok())
            .zip(CTORS)
            .map(|(n, d)| d(n))
            .sum();

        Self::Output {
            duration: total,
            tail: T::response(buf),
        }
    }
}

impl IntoResponse for LinkRequest<'_> {
    type Output = VidId;
    fn response(buf: &mut Vec<&str>) -> Self::Output {
        VidId(buf.pop().unwrap().into())
    }
}

pub trait Response: sealed::Sealed {
    fn id(&self) -> &str;
}

impl<R: Response> Response for Title<R> {
    fn id(&self) -> &str {
        self.tail.id()
    }
}

impl<R: Response> Response for Duration<R> {
    fn id(&self) -> &str {
        self.tail.id()
    }
}

impl Response for VidId {
    fn id(&self) -> &str {
        &self.0
    }
}
