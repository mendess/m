use super::*;

pub trait GetId: sealed::Sealed {
    fn id(&self) -> &VideoId;
}

impl GetId for Box<VideoId> {
    fn id(&self) -> &VideoId {
        self
    }
}

#[trait_gen(T -> Title<R>, Duration<R>, Thumbnail<R>)]
impl<R: GetId> GetId for T {
    fn id(&self) -> &VideoId {
        self.tail.id()
    }
}

pub trait GetTitle: sealed::Sealed {
    fn title(self) -> String;

    fn title_ref(&self) -> &str;
}

impl<X> GetTitle for Title<X> {
    fn title(self) -> String {
        self.title
    }

    fn title_ref(&self) -> &str {
        &self.title
    }
}

#[trait_gen(T -> Duration<R>, Thumbnail<R>)]
impl<R: GetTitle> GetTitle for T {
    fn title(self) -> String {
        self.tail.title()
    }

    fn title_ref(&self) -> &str {
        self.tail.title_ref()
    }
}

pub trait GetDuration: sealed::Sealed {
    fn duration(&self) -> std::time::Duration;
}

impl<X> GetDuration for Duration<X> {
    fn duration(&self) -> std::time::Duration {
        self.duration
    }
}

#[trait_gen(T -> Title<R>, Thumbnail<R>)]
impl<R: GetDuration> GetDuration for T {
    fn duration(&self) -> std::time::Duration {
        self.tail.duration()
    }
}

pub trait GetThumbnail: sealed::Sealed {
    fn thumbnail(&self) -> &str;
}

impl<X> GetThumbnail for Thumbnail<X> {
    fn thumbnail(&self) -> &str {
        &self.thumb
    }
}

#[trait_gen(T -> Title<R>, Duration<R>)]
impl<R: GetThumbnail> GetThumbnail for T {
    fn thumbnail(&self) -> &str {
        self.tail.thumbnail()
    }
}
