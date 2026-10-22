use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::api::{BaseUrl, Client};
use crate::ui::{Failure, Request, Response};

/// Runs the overlay's requests on the API, each in a task of its own, and
/// sends the answers back to the loop.
pub struct Requests {
    client: Option<Client>,
    key_path: PathBuf,
    base_url: BaseUrl,
    answers: mpsc::UnboundedSender<Answer>,
    tasks: JoinSet<()>,
}

#[derive(Debug)]
pub struct Answer {
    pub request: Request,
    pub result: Result<Response, Failure>,
}

impl Requests {
    pub fn new(key_path: PathBuf, base_url: BaseUrl) -> (Self, mpsc::UnboundedReceiver<Answer>) {
        let (answers, receiver) = mpsc::unbounded_channel();
        (
            Self {
                client: None,
                key_path,
                base_url,
                answers,
                tasks: JoinSet::new(),
            },
            receiver,
        )
    }

    /// Starts a request.
    ///
    /// Without a client yet, it first reads the device key and builds one.
    /// If that fails, the request is answered with the failure right away:
    /// [`Failure::SignedOut`] when there is no key, [`Failure::Key`] or
    /// [`Failure::Api`] otherwise.
    pub fn send(&mut self, request: Request) {
        if self.client.is_none() {
            match crate::api::DeviceKey::load(&self.key_path)
                .map_err(Failure::from)
                .and_then(|key| Client::new(self.base_url.clone(), key).map_err(Failure::from))
            {
                Ok(client) => self.client = Some(client),
                Err(error) => {
                    let _ = self.answers.send(Answer {
                        request,
                        result: Err(error),
                    });
                    return;
                }
            }
        }
        let client = self.client.as_ref().expect("client was loaded").clone();
        let answers = self.answers.clone();
        self.tasks.spawn(async move {
            let result = execute(&client, &request).await.map_err(Failure::from);
            let _ = answers.send(Answer { request, result });
        });
    }

    /// Forgets the client, after the key turned out not to be approved, so
    /// that the next request reads the key again.
    pub fn sign_out(&mut self) {
        self.client = None;
    }

    /// Waits for a task to end, and fails if it panicked. Pending forever
    /// while no task runs, so it can sit in `select!`.
    pub async fn join_next(&mut self) -> Result<(), tokio::task::JoinError> {
        if self.tasks.is_empty() {
            return std::future::pending().await;
        }
        match self.tasks.join_next().await {
            Some(result) => result,
            None => Ok(()),
        }
    }
}

async fn execute(client: &Client, request: &Request) -> Result<Response, crate::api::ApiError> {
    match request {
        Request::Profile(username) => client
            .profile(username)
            .await
            .map(|profile| Response::Profile(Box::new(profile))),
        Request::UserCommits { username, cursor } => client
            .user_commits(username, cursor.as_ref())
            .await
            .map(Response::Entries),
        Request::Follow(username) => client.follow(username).await.map(|()| Response::Done),
        Request::Me => client.me().await.map(Response::CurrentUser),
        Request::Log { kind, cursor } => client
            .log(*kind, cursor.as_ref())
            .await
            .map(Response::Entries),
        Request::Notifications { cursor } => client
            .notifications(cursor.as_ref())
            .await
            .map(Response::Notifications),
        Request::Thread(id) | Request::Moderation(id) => {
            client.thread(*id).await.map(Response::Thread)
        }
        Request::Replies { id, cursor } => client
            .replies(*id, cursor.as_ref())
            .await
            .map(Response::Entries),
        Request::NewestReplies { id, cursor } => client
            .newest_replies(*id, cursor.as_ref())
            .await
            .map(Response::Entries),
        Request::Create(commit) => client.create_commit(commit).await.map(Response::Commit),
        Request::Delete(id) => client.delete_commit(*id).await.map(|()| Response::Done),
        Request::Star { id, starred } => client.set_star(*id, *starred).await.map(Response::Commit),
    }
}
