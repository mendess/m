pub mod util;

use std::{
    ops::{Deref, DerefMut},
    process::{ExitStatus, Stdio},
};

use tokio::{
    io::{self, BufReader},
    process::{Child, ChildStdout, Command},
};

use crate::{Link, LinkId, Search};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("status {status_code}, because: {stderr}")]
    YtdlFailed {
        status_code: ExitStatus,
        stderr: String,
    },
    #[error("invalid utf8 {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
}

mod sealed {
    pub trait Sealed {}
    impl<T> Sealed for super::Title<T> {}
    impl<T> Sealed for super::Duration<T> {}
    impl<T> Sealed for super::ThumbnailRequest<T> {}
    impl Sealed for super::VidId {}
    impl<T> Sealed for super::TitleRequest<T> {}
    impl<T> Sealed for super::DurationRequest<T> {}
    impl Sealed for super::LinkRequest<'_> {}
    impl<T> Sealed for super::Thumbnail<T> {}
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

#[repr(transparent)]
pub struct LinkQuery(str);
impl<'l> From<&'l Link> for &'l LinkQuery {
    fn from(l: &'l Link) -> Self {
        unsafe { std::mem::transmute(l.as_str()) }
    }
}
impl<'s> From<&'s Search> for &'s LinkQuery {
    fn from(s: &'s Search) -> Self {
        unsafe { std::mem::transmute(s.as_str().trim_start_matches("ytdl://")) }
    }
}

pub struct LinkRequest<'a>(&'a LinkQuery);
pub struct VidId(String);

pub struct ThumbnailRequest<T>(T);
pub struct Thumbnail<T> {
    thumb: String,
    tail: T,
}

pub struct YtdlBuilder<T>(T);

impl<'l> YtdlBuilder<LinkRequest<'l>> {
    pub fn new<L: Into<&'l LinkQuery>>(link: L) -> Self {
        Self(LinkRequest(link.into()))
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

    pub fn get_thumbnail(self) -> YtdlBuilder<ThumbnailRequest<T>> {
        YtdlBuilder(ThumbnailRequest(self.0))
    }
}

impl<'l, Y, T: YtdlParam<'l> + IntoResponse<Output = Y>> YtdlBuilder<T> {
    pub async fn request(self) -> Result<Ytdl<Y>, Error> {
        let mut v = Vec::new();
        let link = self.0.link();
        v.push(&link.0);
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

// single, expect 3
impl Ytdl<Title<VidId>> {
    pub fn title(self) -> String {
        self.0.title
    }
}

impl Ytdl<Duration<VidId>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }
}

impl Ytdl<Thumbnail<VidId>> {
    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }
}

// double, expect 6

impl Ytdl<Title<Duration<VidId>>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }
}

impl Ytdl<Duration<Title<VidId>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }
}

impl Ytdl<Thumbnail<Title<VidId>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }
}

impl Ytdl<Title<Thumbnail<VidId>>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.thumb
    }
}

impl Ytdl<Thumbnail<Duration<VidId>>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }
}

impl Ytdl<Duration<Thumbnail<VidId>>> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.thumb
    }
}

// triple, expect 6

impl Ytdl<Thumbnail<Title<Duration<VidId>>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.tail.duration
    }
}

impl Ytdl<Thumbnail<Duration<Title<VidId>>>> {
    pub fn title(self) -> String {
        self.0.tail.tail.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }
}

impl Ytdl<Duration<Thumbnail<Title<VidId>>>> {
    pub fn title(self) -> String {
        self.0.tail.tail.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.thumb
    }
}

impl Ytdl<Duration<Title<Thumbnail<VidId>>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.tail.thumb
    }
}

impl Ytdl<Title<Duration<Thumbnail<VidId>>>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.tail.thumb
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }
}

impl Ytdl<Title<Thumbnail<Duration<VidId>>>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.thumb
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.tail.duration
    }
}

impl<R: Response> Ytdl<R> {
    pub fn id(&self) -> &LinkId {
        LinkId::new(self.0.id())
    }
}

pub trait YtdlParam<'l>: sealed::Sealed {
    fn collect(buf: &mut Vec<&str>);
    fn link(&self) -> &'l LinkQuery;
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for TitleRequest<T> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-title");
        T::collect(buf);
    }
    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for DurationRequest<T> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-duration");
        T::collect(buf);
    }
    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for ThumbnailRequest<T> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-thumbnail");
        T::collect(buf);
    }
    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }
}

impl<'l> YtdlParam<'l> for LinkRequest<'l> {
    fn collect(buf: &mut Vec<&str>) {
        buf.push("--get-id")
    }
    fn link(&self) -> &'l LinkQuery {
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
            title: buf.remove(0).to_string(),
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

impl<Y, T: IntoResponse<Output = Y>> IntoResponse for ThumbnailRequest<T> {
    type Output = Thumbnail<Y>;

    fn response(buf: &mut Vec<&str>) -> Self::Output {
        let i = buf.len().saturating_sub(1).saturating_sub(
            buf.iter()
                .rev()
                .position(|e| e.starts_with("http"))
                .unwrap(),
        );
        Self::Output {
            thumb: buf.remove(i).to_string(),
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

pub struct StreamingChild {
    _child: Child,
    pub stdout: BufReader<ChildStdout>,
}

impl Deref for StreamingChild {
    type Target = BufReader<ChildStdout>;

    fn deref(&self) -> &Self::Target {
        &self.stdout
    }
}

impl DerefMut for StreamingChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stdout
    }
}

pub async fn get_playlist_video_ids(link: &str) -> Result<StreamingChild, Error> {
    let mut child = Command::new("youtube-dl")
        .arg("--get-id")
        .arg(link)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .spawn()?;

    Ok(StreamingChild {
        stdout: BufReader::new(child.stdout.take().unwrap()),
        _child: child,
    })
}
