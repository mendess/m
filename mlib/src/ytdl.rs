mod getters;
pub mod util;

use std::{
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

use crate::{Error, Search, Link, LinkId};

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

impl<'l, Y: 'static, T: YtdlParam<'l> + IntoResponse<Output = Y>> YtdlBuilder<T> {
    pub async fn request(self) -> Result<Ytdl<Y>, Error> {
        let n_fields = self.0.n_params();
        match self.request_multiple()?.next().await {
            Some(r) => r,
            None => Err(YtdlError::InsufisientFields {
                expected: n_fields,
                found: 0,
                fields: vec![],
            }
            .into()),
        }
    }

    pub fn request_multiple(&self) -> Result<YtdlStream2<Y>, Error> {
        let mut cmd = Command::new("youtube-dl");
        cmd.arg(&self.0.link().0);
        T::collect(&mut cmd);

        let mut child = cmd.kill_on_drop(true).stdout(Stdio::piped()).spawn()?;

        let n_fields = self.0.n_params();

        Ok(YtdlStream2 {
            stream: LinesStream::new(BufReader::new(child.stdout.take().unwrap()).lines())
                .chunks(n_fields),
            n_fields,
            response: T::response,
            _child: child,
        })
    }
}

#[pin_project]
pub struct YtdlStream2<Y> {
    #[pin]
    stream: Chunks<LinesStream<BufReader<ChildStdout>>>,
    n_fields: usize,
    response: fn(&mut Vec<String>) -> Y,
    _child: Child,
}

impl<Y> Stream for YtdlStream2<Y> {
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
    pub fn id(&self) -> &LinkId {
        LinkId::new(self.0.id())
    }
}

pub trait YtdlParam<'l>: sealed::Sealed {
    fn collect(cmd: &mut Command);
    fn link(&self) -> &'l LinkQuery;
    fn n_params(&self) -> usize;
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for TitleRequest<T> {
    fn collect(cmd: &mut Command) {
        cmd.arg("--get-title");
        T::collect(cmd);
    }

    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for DurationRequest<T> {
    fn collect(cmd: &mut Command) {
        cmd.arg("--get-duration");
        T::collect(cmd);
    }

    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l, T: YtdlParam<'l>> YtdlParam<'l> for ThumbnailRequest<T> {
    fn collect(cmd: &mut Command) {
        cmd.arg("--get-thumbnail");
        T::collect(cmd);
    }

    fn link(&self) -> &'l LinkQuery {
        self.0.link()
    }

    fn n_params(&self) -> usize {
        1 + self.0.n_params()
    }
}

impl<'l> YtdlParam<'l> for LinkRequest<'l> {
    fn collect(cmd: &mut Command) {
        cmd.arg("--get-id");
    }

    fn link(&self) -> &'l LinkQuery {
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

impl IntoResponse for LinkRequest<'_> {
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
