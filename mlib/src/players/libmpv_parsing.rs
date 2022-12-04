use std::any::type_name;

use libmpv::MpvNode;

use super::{
    error::{MpvError, MpvResult},
    QueueItem, QueueItemStatus,
};

pub(super) fn parse_queue_item_status(node: MpvNode) -> MpvResult<QueueItemStatus> {
    let mk_err = |error: &'static str| {
        || MpvError::InvalidData {
            expected: type_name::<QueueItemStatus>().to_string(),
            got: format!("{node:?}"),
            error: error.to_string(),
        }
    };
    node.to_map()
        .ok_or_else(mk_err("wrong node type, expected map"))
        .and_then(|m| {
            let mut current = None;
            let mut playing = None;
            for (k, v) in m {
                match k {
                    "current" => {
                        current = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "playing" => {
                        playing = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    _ => {}
                };
            }
            if let (Some(current), Some(playing)) = (current, playing) {
                Ok(QueueItemStatus { current, playing })
            } else {
                Err(mk_err("missing current or playing from node")())
            }
        })
}

pub(super) fn parse_queue_item(node: MpvNode) -> MpvResult<QueueItem> {
    let mk_err = |error: &'static str| {
        || MpvError::InvalidData {
            expected: type_name::<QueueItem>().to_string(),
            got: format!("{node:?}"),
            error: error.to_string(),
        }
    };
    node.to_map()
        .ok_or_else(mk_err("wrong node type, expected map"))
        .and_then(|i| {
            let mut filename = None;
            let mut status = None;
            let mut current = None;
            let mut playing = None;
            let mut id = None;
            for (k, v) in i {
                match k {
                    "filename" => {
                        filename = Some(
                            v.to_str()
                                .ok_or_else(mk_err("wrong node type, expected string"))?
                                .to_string(),
                        )
                    }
                    "status" => status = Some(parse_queue_item_status(v)?),
                    "current" => {
                        current = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "playing" => {
                        playing = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "id" => {
                        id = Some(
                            v.to_i64()
                                .ok_or_else(mk_err("wrong node type, expected i64"))?
                                as usize,
                        )
                    }
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
                Err(mk_err("missing fields filename or status or id")())
            }
        })
}
