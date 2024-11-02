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
    item::{
        link::{ChannelLink, Id, VideoLink},
        PlaylistLink,
    },
    Error, Search, VideoId,
};
use thiserror::Error;
use trait_gen::trait_gen;

mod sealed {
    use trait_gen::trait_gen;

    pub trait Sealed {}
    #[trait_gen(U ->
        super::Title<T>,        super::Duration<T>,        super::Thumbnail<T>,
        super::TitleRequest<T>, super::DurationRequest<T>, super::ThumbnailRequest<T>,
        super::LinkRequest<'_, T>,
    )]
    impl<T> Sealed for U {}
    #[trait_gen(T ->
        crate::item::VideoLink,   crate::item::PlaylistLink,
        crate::item::ChannelLink, crate::item::Search,
        Box<crate::item::VideoId>,
    )]
    impl Sealed for T {}
}

pub trait YtdlParam<'l>: sealed::Sealed {
    type Link;
    fn collect(cmd: &mut Command);
    fn link_and_param_count(&self) -> (&'l Self::Link, usize);
}

pub trait IntoResponse: sealed::Sealed {
    type Output: ?Sized + 'static;
    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>;
}

macro_rules! impl_request {
    ($name:ident => $output:ident with $arg:literal == $field:ident : |$buf:ident| $transform:block) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name<T>(T);
        impl<'l, L, T: YtdlParam<'l, Link = L>> YtdlParam<'l> for $name<T> {
            type Link = L;

            fn collect(cmd: &mut tokio::process::Command) {
                cmd.arg($arg);
                T::collect(cmd);
            }

            fn link_and_param_count(&self) -> (&'l Self::Link, usize) {
                let (link, count) = self.0.link_and_param_count();
                (link, 1 + count)
            }
        }

        impl<Y: 'static, T: IntoResponse<Output = Y>> IntoResponse for $name<T> {
            type Output = $output<Y>;
            fn response<S>($buf: &mut Vec<S>) -> Self::Output
            where
                S: AsRef<str>,
            {
                Self::Output {
                    $field: $transform,
                    tail: T::response($buf),
                }
            }
        }
    };
}

impl_request!(TitleRequest => Title with "--get-title" == title: |buf| {
    buf.remove(0).as_ref().to_string()
});

impl_request!(DurationRequest => Duration with "--get-duration" == duration: |buf| {
    use std::time::Duration;

    const CTORS: &[fn(u64) -> Duration] = &[
        Duration::from_secs,
        |m| Duration::from_secs(m * 60),
        |h| Duration::from_secs(h * 60 * 60),
    ];

    let dur = buf.pop().unwrap();
    dur
        .as_ref()
        .split(':')
        .rev()
        .map_while(|n| n.parse::<u64>().ok())
        .zip(CTORS)
        .map(|(n, d)| d(n))
        .sum()
});

impl_request!(ThumbnailRequest => Thumbnail with "--get-thumbnail" == thumb: |buf| {
    let i = buf.len().saturating_sub(1).saturating_sub(
        buf.iter()
            .rev()
            .position(|e| e.as_ref().starts_with("http"))
            .unwrap(),
    );
    buf.remove(i).as_ref().to_string()
});

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LinkRequest<'l, L>(&'l L);

impl<'l, L> YtdlParam<'l> for LinkRequest<'l, L> {
    type Link = L;

    fn collect(cmd: &mut Command) {
        cmd.arg("--get-id");
    }

    fn link_and_param_count(&self) -> (&'l Self::Link, usize) {
        (self.0, 1)
    }
}

impl<L> IntoResponse for LinkRequest<'_, L> {
    type Output = Box<VideoId>;
    fn response<S>(buf: &mut Vec<S>) -> Self::Output
    where
        S: AsRef<str>,
    {
        VideoId::new(buf.pop().unwrap().as_ref()).boxed()
    }
}

#[trait_gen(T -> ChannelLink, VideoLink, PlaylistLink, Search)]
impl<'l> From<&'l T> for LinkRequest<'l, T> {
    fn from(l: &'l T) -> Self {
        Self(l)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct YtdlBuilder<T>(T);

impl<'l, L> YtdlBuilder<LinkRequest<'l, L>>
where
    LinkRequest<'l, L>: From<&'l L>,
{
    pub fn new(link: &'l L) -> Self {
        Self(link.into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Ytdl<T: ?Sized>(T);

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
        let (link, n_fields) = self.0.link_and_param_count();
        request_impl::<_, T, _>(link, n_fields)?
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
        let (link, n_fields) = self.0.link_and_param_count();
        request_impl::<_, T, _>(link.as_str().trim_start_matches("ytdl://"), n_fields)?
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

    pub fn search_multiple(&self) -> Result<YtdlStream<Y>, Error> {
        let (link, n_fields) = self.0.link_and_param_count();
        request_impl::<&str, T, Y>(link.as_str().trim_start_matches("ytdl://"), n_fields)
    }
}

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = PlaylistLink>,
{
    pub fn request_playlist(&self) -> Result<YtdlStream<Y>, Error> {
        let (link, n_fields) = self.0.link_and_param_count();
        request_impl::<_, T, _>(link.without_video_id(), n_fields)
    }
}

impl<'l, Y, T> YtdlBuilder<T>
where
    T: IntoResponse<Output = Y>,
    T: YtdlParam<'l, Link = ChannelLink>,
{
    pub fn request_channel(&self) -> Result<YtdlStream<Y>, Error> {
        let (link, n_fields) = self.0.link_and_param_count();
        request_impl::<_, T, _>(link, n_fields)
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
    tracing::debug!(args = ?cmd.as_std().get_args(), "running ytdl");

    let mut child = cmd.kill_on_drop(true).stdout(Stdio::piped()).spawn()?;

    Ok(YtdlStream {
        stream: LinesStream::new(BufReader::new(child.stdout.take().unwrap()).lines())
            .chunks(n_fields),
        n_fields,
        response: T::response,
        _child: child,
    })
}

#[derive(Debug)]
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

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Title<T: ?Sized> {
    title: String,
    tail: T,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Duration<T: ?Sized> {
    duration: std::time::Duration,
    tail: T,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Thumbnail<T: ?Sized> {
    thumb: String,
    tail: T,
}

impl<R: getters::GetId> Ytdl<R> {
    pub fn id(&self) -> &VideoId {
        self.0.id()
    }
}

impl<R: getters::GetTitle> Ytdl<R> {
    pub fn title(self) -> String {
        self.0.title()
    }

    pub fn title_ref(&self) -> &str {
        self.0.title_ref()
    }
}

impl<R: getters::GetDuration> Ytdl<R> {
    pub fn duration(&self) -> std::time::Duration {
        self.0.duration()
    }
}

impl<R: getters::GetThumbnail> Ytdl<R> {
    pub fn thumbnail(&self) -> &str {
        self.0.thumbnail()
    }
}
