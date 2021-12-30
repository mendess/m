use super::*;

// single, expect 3
impl Ytdl<Title<VidId>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn title_ref(&self) -> &str {
        &self.0.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.duration
    }
}

impl Ytdl<Duration<Title<VidId>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn title_ref(&self) -> &str {
        &self.0.tail.title
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.duration
    }
}

impl Ytdl<Thumbnail<Title<VidId>>> {
    pub fn title(self) -> String {
        self.0.tail.title
    }

    pub fn title_ref(&self) -> &str {
        &self.0.tail.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.thumb
    }
}

impl Ytdl<Title<Thumbnail<VidId>>> {
    pub fn title(self) -> String {
        self.0.title
    }

    pub fn title_ref(&self) -> &str {
        &self.0.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.tail.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.tail.tail.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.tail.tail.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.tail.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.title
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

    pub fn title_ref(&self) -> &str {
        &self.0.title
    }

    pub fn thumbnail(&self) -> &str {
        &self.0.tail.thumb
    }

    pub fn duration(&self) -> std::time::Duration {
        self.0.tail.tail.duration
    }
}
