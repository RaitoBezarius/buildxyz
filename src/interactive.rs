use std::thread;
use std::{
    sync::mpsc::{channel, Sender},
    thread::JoinHandle,
};

use log::{debug, info, warn};

use crate::cache::{FileTreeEntry, StorePath};
use crate::fs::FsEventMessage;

/// Request types between FUSE thread and UI thread
pub enum UserRequest {
    /// Order the thread to stop listen for events
    Quit,
    /// An interactive search request for the given path to the UI thread
    /// with a preferred candidate.
    InteractiveSearch(Vec<(StorePath, FileTreeEntry)>, (StorePath, FileTreeEntry)),
}

pub fn prompt_among_choices(
    prompt: &str,
    choices: Vec<String>
) -> Option<usize> {
    loop {
        let mut answer = String::new();
        info!("{}", prompt);
        for (index, choice) in choices.iter().enumerate() {
            info!("{}. {}", index + 1, choice);
        }
        // TODO: make this non-blocking and interruptible
        std::io::stdin()
            .read_line(&mut answer)
            .ok()
            .expect("Failed to read line");

        if answer.trim().to_lowercase() == "n" || answer.trim().to_lowercase() == "no" || answer.trim() == "" {
            return None;
        }

        match answer.trim().parse::<usize>() {
            Ok(k) if k >= 1 && k <= choices.len() => {
                return Some(k - 1);
            }
            _ => {
                warn!("Enter a valid choice between 1 and {} or `no`/`n`/press enter for skipping this choice", choices.len());
                continue;
            }
        }
    }
}

pub fn spawn_ui(
    reply_fs: Sender<FsEventMessage>,
    automatic: bool,
) -> (JoinHandle<()>, Sender<UserRequest>) {
    let (send, recv) = channel();

    let join_handle = thread::spawn(move || {
        info!("UI thread spawned and listening for events");
        loop {
            if let Ok(message) = recv.recv() {
                match message {
                    UserRequest::Quit => {
                        break;
                    }
                    UserRequest::InteractiveSearch(candidates, suggested) => {
                        if automatic {
                            reply_fs
                                .send(FsEventMessage::PackageSuggestion(suggested))
                                .expect("Failed to send message to FS thread");
                            continue;
                        }

                        let choices: Vec<String> = candidates.iter().map(|(c, _)| c.origin().as_ref().clone().attr).collect();
                        let potential_index = prompt_among_choices(
                            "A dependency not found in your search paths was requested, pick a choice",
                            choices
                        );

                        match potential_index {
                            Some(index) => reply_fs.send(FsEventMessage::PackageSuggestion(candidates[index].clone())),
                            None => reply_fs.send(FsEventMessage::IgnorePendingRequests),
                        }
                        .expect("Failed to send message to FS thread");

                        // list all the candidates with numbers
                        // provide ENOENT option

                        // ENOENT
                    }
                }
            }
        }
    });

    (join_handle, send)
}
