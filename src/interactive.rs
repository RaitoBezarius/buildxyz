use std::thread;
use std::{thread::JoinHandle, sync::mpsc::{Sender, channel}};

use log::{info, debug};

use crate::cache::{StorePath, FileTreeEntry};
use crate::fs::FsEventMessage;

/// Request types between FUSE thread and UI thread
pub enum UserRequest {
    /// Order the thread to stop listen for events
    Quit,
    /// An interactive search request for the given path to the UI thread
    /// with a preferred candidate.
    InteractiveSearch(Vec<(StorePath, FileTreeEntry)>, StorePath),
}

pub fn spawn_ui(reply_fs: Sender<FsEventMessage>) -> (JoinHandle<()>, Sender<UserRequest>) {
    let (send, recv) = channel();

    let join_handle = thread::spawn(move || {
        info!("UI thread spawned and listening for events");
        loop {
            match recv.recv().expect("Failed to receive message") {
                UserRequest::Quit => { break; },
                UserRequest::InteractiveSearch(_candidates, _suggested) => {
                    // prompt the user with the suggestion: yes/no?
                    // list all the candidates with numbers
                    // provide ENOENT option

                    // ENOENT
                    reply_fs.send(FsEventMessage::IgnorePendingRequests).expect("Failed to send message to FS thread");
                }
            }
        }
    });

    (join_handle, send)
}
