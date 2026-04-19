#![allow(dead_code)]

use commanductui::types::TreeItemId;

const LINK_FLAG: u64 = 0x8000_0000_0000_0000;
const FOLDER_FLAG: u64 = 0x4000_0000_0000_0000;
const SHOW_MORE_FLAG: u64 = 0x2000_0000_0000_0000;
const JOB_MASK: u64 = !(LINK_FLAG | FOLDER_FLAG | SHOW_MORE_FLAG);

pub fn job_tree_item_id(job_id: u64) -> TreeItemId {
    TreeItemId::new(job_id & JOB_MASK)
}

pub fn links_folder_tree_item_id(job_id: u64) -> TreeItemId {
    TreeItemId::new(FOLDER_FLAG | (job_id & JOB_MASK))
}

pub fn link_tree_item_id(job_id: u64, link_index: u32) -> TreeItemId {
    let job_component = (job_id & 0x7FFF_FFFF) << 32;
    TreeItemId::new(LINK_FLAG | job_component | link_index as u64)
}

pub fn links_show_more_tree_item_id(job_id: u64) -> TreeItemId {
    TreeItemId::new(SHOW_MORE_FLAG | (job_id & JOB_MASK))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeItemKind {
    Job { job_id: u64 },
    LinksFolder { job_id: u64 },
    Link { job_id: u64, link_index: u32 },
    LinksShowMore { job_id: u64 },
}

pub fn decode_tree_item_id(id: TreeItemId) -> TreeItemKind {
    let raw = id.raw();
    if raw & LINK_FLAG != 0 {
        TreeItemKind::Link {
            job_id: (raw >> 32) & 0x7FFF_FFFF,
            link_index: (raw & 0xFFFF_FFFF) as u32,
        }
    } else if raw & FOLDER_FLAG != 0 {
        TreeItemKind::LinksFolder {
            job_id: raw & JOB_MASK,
        }
    } else if raw & SHOW_MORE_FLAG != 0 {
        TreeItemKind::LinksShowMore {
            job_id: raw & JOB_MASK,
        }
    } else {
        TreeItemKind::Job {
            job_id: raw & JOB_MASK,
        }
    }
}
