use std::thread;
use std::{
    sync::mpsc::{channel, Sender},
    thread::JoinHandle,
};

use log::{debug, info};

use crate::cache::{FileTreeEntry, StorePath};
use crate::fs::FsEventMessage;

/// Request types between FUSE thread and UI thread
pub enum UserRequest {
    /// Order the thread to stop listen for events
    Quit,
    /// An interactive search request for the given path to the UI thread
    /// with a preferred candidate.
    InteractiveSearch(Vec<(StorePath, FileTreeEntry)>, StorePath),
}

pub fn spawn_ui(
    reply_fs: Sender<FsEventMessage>,
    automatic: bool,
) -> (JoinHandle<()>, Sender<UserRequest>) {
    let (send, recv) = channel();

    let join_handle = thread::spawn(move || {
        info!("UI thread spawned and listening for events");
        loop {
            match recv.recv().expect("Failed to receive message") {
                UserRequest::Quit => {
                    break;
                }
                UserRequest::InteractiveSearch(_candidates, suggested) => {
                    if automatic {
                        reply_fs
                            .send(FsEventMessage::PackageSuggestion(suggested))
                            .expect("Failed to send message to FS thread");
                        continue;
                    }

                    let mut answer = String::new();
                    info!(
                        "Dependency requested, suggestion is `{}`, inject it? y/n",
                        suggested.origin().attr
                    );
                    std::io::stdin()
                        .read_line(&mut answer)
                        .ok()
                        .expect("Failed to read line");

                    match answer.as_str().trim() {
                        "y" | "yes" | "Y" => {
                            reply_fs.send(FsEventMessage::PackageSuggestion(suggested))
                        }
                        _ => reply_fs.send(FsEventMessage::IgnorePendingRequests),
                    }
                    .expect("Failed to send message to FS thread");

                    // list all the candidates with numbers
                    // provide ENOENT option

                    // ENOENT
                }
            }
        }
    });

    (join_handle, send)
}
