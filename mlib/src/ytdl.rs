mod getters;
pub mod util;

use std::{
    ffi::OsStr,
    pin::Pin,
    process::{ExitStatus, Stdio},
    task::{Context, Poll},
};

use futures_util::stream::{Chunks, Stream, StreamExt};
use pin_project::pin_project;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdout, Command},
};
use tokio_stream::wrappers::LinesStream;

use crate::{
    item::{link::VideoLink, PlaylistLink},
    Error, Link, Search, VideoId,
};
use thiserror::Error;

mod sealed {
    pub trait Sealed {}
    impl<T> Sealed for super::Title<T> {}
    impl<T> Sealed for super::Duration<T> {}
    impl<T> Sealed for super::ThumbnailRequest<T> {}
    impl Sealed for super::VidId {}
    impl<T> Sealed for super::TitleRequest<T> {}
    impl<T> Sealed for super::DurationRequest<T> {}
    impl<L> Sealed for super::LinkRequest<'_, L> {}
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

impl<'l> From<&'l Link> for LinkRequest<'l, Link> {
    fn from(l: &'l Link) -> Self {
        Self(l)
    }
}

impl<'l> From<&'l VideoLink> for LinkRequest<'l, VideoLink> {
    fn from(l: &'l VideoLink) -> Self {
        Self(l)
    }
}
impl<'l> From<&'l PlaylistLink> for LinkRequest<'l, PlaylistLink> {
    fn from(l: &'l PlaylistLink) -> Self {
        Self(l)
    }
}
impl<'s> From<&'s Search> for LinkRequest<'s, Search> {
    fn from(s: &'s Search) -> Self {
        Self(s)
        // unsafe { std::mem::transmute(s.as_str().trim_start_matches("ytdl://")) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LinkRequest<'l, L>(&'l L);
pub struct VidId(String);

pub struct ThumbnailRequest<T>(T);
pub struct Thumbnail<T> {
    thumb: String,
    tail: T,
}

pub struct YtdlBuilder<T>(T);

impl<'l, L> YtdlBuilder<LinkRequest<'l, L>>
where
    LinkRequest<'l, L>: From<&'l L>,
{
    pub fn new(link: &'l L) -> Self {
        Self(link.into())
    }
}

pub struct Ytdl<T>(T);

#[derive(Error, Debug)]
pub enum YtdlError {
    #[error("not enough fields, expected {expected} found {found}: {fields:?}")]
    InsufisientFields {
        expected: usize,
        found: usize,
        fields: Vec<String>,
    },
    #[error("status {status_code}, because: {stderr}")]
    NonZeroStatus {
        status_code: ExitStatus,
        stderr: String,
    },
}

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

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = VideoLink>,
{
    pub async fn request(self) -> Result<Ytdl<Y>, Error> {
        let n_fields = self.0.n_params();
        let link = self.0.link();
        request_impl::<_, T, _>(link.as_ref(), n_fields)?
            .next()
            .await
            .ok_or_else(|| {
                Error::from(YtdlError::InsufisientFields {
                    expected: n_fields,
                    found: 0,
                    fields: vec![],
                })
            })?
    }
}

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = Search>,
{
    pub async fn search(self) -> Result<Ytdl<Y>, Error> {
        let n_fields = self.0.n_params();
        request_impl::<_, T, _>(
            self.0.link().as_str().trim_start_matches("ytdl://"),
            n_fields,
        )?
        .next()
        .await
        .ok_or_else(|| {
            Error::from(YtdlError::InsufisientFields {
                expected: n_fields,
                found: 0,
                fields: vec![],
            })
        })?
    }
}

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = PlaylistLink>,
{
    pub fn request_playlist(&self) -> Result<YtdlStream<Y>, Error> {
        request_impl::<_, T, _>(&self.0.link().without_video_id(), self.0.n_params())
    }
}

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = Search>,
{
    pub fn search_multiple(&self) -> Result<YtdlStream<Y>, Error> {
        request_impl::<&str, T, Y>(
            self.0.link().as_str().trim_start_matches("ytdl://"),
            self.0.n_params(),
        )
    }
}

fn request_impl<'l, L, T, Y>(link: L, n_fields: usize) -> Result<YtdlStream<Y>, Error>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l>,
    L: AsRef<OsStr>,
{
    let mut cmd = Command::new("yt-dlp");
    cmd.arg(link);
    T::collect(&mut cmd);

    let mut child = cmd.kill_on_drop(true).stdout(Stdio::piped()).spawn()?;

    Ok(YtdlStream {
        stream: LinesStream::new(BufReader::new(child.stdout.take().unwrap()).lines())
            .chunks(n_fields),
        n_fields,
        response: T::response,
        _child: child,
    })
}

#[pin_project]
pub struct YtdlStream<Y> {
    #[pin]
    stream: Chunks<LinesStream<BufReader<ChildStdout>>>,
    n_fields: usize,
    response: fn(&mut Vec<String>) -> Y,
    _child: Child,
}

impl<Y> Stream for YtdlStream<Y> {
    type Item = Result<Ytdl<Y>, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let n_fields = self.n_fields;
        let response = self.response;
        match self.project().stream.poll_next(cx) {
            Poll::Ready(Some(lines)) => Poll::Ready(Some({
                let lines = lines
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(Error::from);
                lines.and_then(|mut lines| {
                    if lines.len() == n_fields {
                        Ok(Ytdl(response(&mut lines)))
                    } else {
                        Err(Error::from(YtdlError::InsufisientFields {
                            expected: n_fields,
                            found: lines.len(),
                            fields: lines,
                        }))
                    }
                })
            })),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R: Response> Ytdl<R> {
    pub fn id(&self) -> &VideoId {
        VideoId::new(self.0.id())
    }
}

pub trait YtdlParam<'l>: sealed::Sealed {
    type Link;
    fn collect(cmd: &mut Command);
    fn link(&self) -> &'l Self::Link;
    fn n_params(&self) -> usize;
}

impl<'l, L, T: YtdlParam<'l, Link = L>> YtdlParam<'l> for TitleRequest<T> {
    type Link = L;

    fn collect(cmd: &mut Command) {
        cmd.arg("--get-title");
        T::collect(cmd);
    }

    fn link(&self) -> &'l Self::Link {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l, L, T: YtdlParam<'l, Link = L>> YtdlParam<'l> for DurationRequest<T> {
    type Link = L;

    fn collect(cmd: &mut Command) {
        cmd.arg("--get-duration");
        T::collect(cmd);
    }

    fn link(&self) -> &'l Self::Link {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l, L, T: YtdlParam<'l, Link = L>> YtdlParam<'l> for ThumbnailRequest<T> {
    type Link = L;

    fn collect(cmd: &mut Command) {
        cmd.arg("--get-thumbnail");
        T::collect(cmd);
    }

    fn link(&self) -> &'l Self::Link {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l, L> YtdlParam<'l> for LinkRequest<'l, L> {
    type Link = L;

    fn collect(cmd: &mut Command) {
        cmd.arg("--get-id");
    }

    fn link(&self) -> &'l Self::Link {
        self.0
    }

    fn n_params(&self) -> usize {
        1
    }
}

pub trait IntoResponse: sealed::Sealed {
    type Output: 'static;
    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>;
}

impl<Y: 'static, T: IntoResponse<Output = Y>> IntoResponse for TitleRequest<T> {
    type Output = Title<Y>;
    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>,
    {
        Self::Output {
            title: buf.remove(0).as_ref().to_string(),
            tail: T::response(buf),
        }
    }
}

impl<Y: 'static, T: IntoResponse<Output = Y>> IntoResponse for DurationRequest<T> {
    type Output = Duration<Y>;

    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>,
    {
        use std::time::Duration;

        const CTORS: &[fn(u64) -> Duration] = &[
            Duration::from_secs,
            |m| Duration::from_secs(m * 60),
            |h| Duration::from_secs(h * 60 * 60),
        ];

        let dur = buf.pop().unwrap();
        let total = dur
            .as_ref()
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

impl<Y: 'static, T: IntoResponse<Output = Y>> IntoResponse for ThumbnailRequest<T> {
    type Output = Thumbnail<Y>;

    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>,
    {
        let i = buf.len().saturating_sub(1).saturating_sub(
            buf.iter()
                .rev()
                .position(|e| e.as_ref().starts_with("http"))
                .unwrap(),
        );
        Self::Output {
            thumb: buf.remove(i).as_ref().to_string(),
            tail: T::response(buf),
        }
    }
}

impl<L> IntoResponse for LinkRequest<'_, L> {
    type Output = VidId;
    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>,
    {
        VidId(buf.pop().unwrap().as_ref().to_string())
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
