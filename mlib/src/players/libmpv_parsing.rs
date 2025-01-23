use std::any::type_name;

use libmpv::{MpvNode, MpvNodeMapIter};

use super::{
    error::{MpvError, MpvResult},
    QueueItem, QueueItemStatus,
};

pub(super) fn parse_queue_item(node: MpvNode) -> MpvResult<QueueItem> {
    parse_node(node)
}

trait Parse: Sized {
    fn parse(m: MpvNodeMapIter<'_>) -> Result<Self, &'static str>;
}

impl Parse for QueueItem {
    fn parse(m: MpvNodeMapIter<'_>) -> Result<Self, &'static str> {
        let mut filename = None;
        let mut status = None;
        let mut current = None;
        let mut playing = None;
        let mut id = None;
        for (k, v) in m {
            match k {
                "filename" => {
                    filename = Some(
                        v.to_str()
                            .ok_or("wrong node type, expected string")?
                            .to_string(),
                    )
                }
                "status" => {
                    status = Some(QueueItemStatus::parse(
                        v.to_map().ok_or("wrong node type, expected map")?,
                    )?)
                }
                "current" => current = Some(v.to_bool().ok_or("wrong node type, expected bool")?),
                "playing" => playing = Some(v.to_bool().ok_or("wrong node type, expected bool")?),
                "id" => id = Some(v.to_i64().ok_or("wrong node type, expected i64")? as usize),
                _ => {}
            };
        }
        status = status.or_else(|| {
            Some(QueueItemStatus {
                current: current?,
                playing: playing?,
            })
        });
        if let (Some(filename), status, Some(id)) = (filename, status, id) {
            Ok(QueueItem {
                filename,
                status,
                id,
            })
        } else {
            Err("missing fields filename or status or id")
        }
    }
}

impl Parse for QueueItemStatus {
    fn parse(m: MpvNodeMapIter<'_>) -> Result<Self, &'static str> {
        let mut current = None;
        let mut playing = None;
        for (k, v) in m {
            match k {
                "current" => current = Some(v.to_bool().ok_or("wrong node type, expected bool")?),
                "playing" => playing = Some(v.to_bool().ok_or("wrong node type, expected bool")?),
                _ => {}
            };
        }
        if let (Some(current), Some(playing)) = (current, playing) {
            Ok(QueueItemStatus { current, playing })
        } else {
            Err("missing current or playing from node")
        }
    }
}

fn parse_node<T: Parse>(node: MpvNode) -> MpvResult<T> {
    let mk_err = |error: &'static str| MpvError::InvalidData {
        expected: type_name::<QueueItem>().to_string(),
        got: format!("{node:?}"),
        error: error.to_string(),
    };
    node.to_map()
        .ok_or_else(|| mk_err("wrong node type, expected map"))
        .and_then(|m| T::parse(m).map_err(mk_err))
}
