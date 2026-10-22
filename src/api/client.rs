use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use url::Url;

use super::{
    ApiError, Commit, CommitEntry, CommitId, CurrentUser, Cursor, DeviceApproval, DeviceKey,
    LogKind, NewCommit, NotificationPage, Page, Profile, Thread, User, Username,
};

/// The /dev/null site to talk to.
///
/// Only `https://` URLs are accepted, and `http://` on `localhost` or
/// `127.0.0.1` for development, as the site's own script allows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BaseUrl(Url);

impl BaseUrl {
    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

/// `https://dev-null.sh`.
impl Default for BaseUrl {
    fn default() -> Self {
        Self(Url::parse("https://dev-null.sh").expect("constant URL"))
    }
}

impl fmt::Display for BaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for BaseUrl {
    type Err = BaseUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(s)?;
        if url.scheme() == "https"
            || (url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1")))
        {
            Ok(Self(url))
        } else {
            Err(BaseUrlError::Insecure(url))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BaseUrlError {
    #[error("not a URL")]
    Parse(#[from] url::ParseError),
    #[error("{0} is not https://, nor http:// on localhost")]
    Insecure(Url),
}

/// Signed requests to the API, as the device that a [`DeviceKey`] stands for.
///
/// Each request is signed once over the final URL's path and query and the
/// exact body bytes, and sent as signed. Redirects are not followed, since
/// they would carry the signature to another path or site.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base: BaseUrl,
    key: Arc<DeviceKey>,
}

impl Client {
    pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    /// For the whole request, answer included.
    pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Fails only if the HTTP client can't be built, such as when the TLS
    /// roots can't be loaded.
    pub fn new(base: BaseUrl, key: DeviceKey) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Self::CONNECT_TIMEOUT)
            .timeout(Self::REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            base,
            key: Arc::new(key),
        })
    }

    pub fn base(&self) -> &BaseUrl {
        &self.base
    }

    pub async fn me(&self) -> Result<CurrentUser, ApiError> {
        self.request("GET", "/api/v1/me", None, None).await
    }

    /// Across a block, /dev/null answers with
    /// [`UserBlocked`](super::ErrorCode::UserBlocked) or
    /// [`UserUnavailable`](super::ErrorCode::UserUnavailable).
    pub async fn profile(&self, username: &Username) -> Result<Profile, ApiError> {
        self.request("GET", &self.user_path(username)?, None, None)
            .await
    }

    /// A page of the user's commits that are not replies, newest first, without
    /// the pinned one.
    pub async fn user_commits(
        &self,
        username: &Username,
        cursor: Option<&Cursor>,
    ) -> Result<Page<CommitEntry>, ApiError> {
        self.request(
            "GET",
            &format!("{}/commits", self.user_path(username)?),
            cursor,
            None,
        )
        .await
    }

    /// Following someone already followed does nothing.
    pub async fn follow(&self, username: &Username) -> Result<(), ApiError> {
        self.raw(
            "POST",
            &format!("{}/follow", self.user_path(username)?),
            None,
            None,
        )
        .await
        .map(|_| ())
    }

    /// A page of notification groups, newest first, unfiltered. As on the web,
    /// this marks every notification as read.
    pub async fn notifications(
        &self,
        cursor: Option<&Cursor>,
    ) -> Result<NotificationPage, ApiError> {
        self.request("GET", "/api/v1/notifications", cursor, None)
            .await
    }

    /// A page of a log, newest first, or ranked for
    /// [`LogKind::Trending`]. The user's own pending and rejected commits are
    /// in it too.
    pub async fn log(
        &self,
        kind: LogKind,
        cursor: Option<&Cursor>,
    ) -> Result<Page<CommitEntry>, ApiError> {
        self.request("GET", &format!("/api/v1/log/{kind}"), cursor, None)
            .await
    }

    pub async fn thread(&self, id: CommitId) -> Result<Thread, ApiError> {
        self.request("GET", &format!("/api/v1/commits/{id}"), None, None)
            .await
    }

    /// A page of the replies to a commit, oldest first.
    pub async fn replies(
        &self,
        id: CommitId,
        cursor: Option<&Cursor>,
    ) -> Result<Page<CommitEntry>, ApiError> {
        self.request(
            "GET",
            &format!("/api/v1/commits/{id}/replies"),
            cursor,
            None,
        )
        .await
    }

    /// A page of replies newest first, for catching up an exhausted thread.
    pub async fn newest_replies(
        &self,
        id: CommitId,
        cursor: Option<&Cursor>,
    ) -> Result<Page<CommitEntry>, ApiError> {
        self.request(
            "GET",
            &format!("/api/v1/commits/{id}/replies?newest=1"),
            cursor,
            None,
        )
        .await
    }

    /// Returns the new commit, pending review unless the user is an admin.
    pub async fn create_commit(&self, commit: &NewCommit) -> Result<Commit, ApiError> {
        self.data(
            "POST",
            "/api/v1/commits",
            Some(serde_json::to_vec(commit).map_err(ApiError::Decode)?),
        )
        .await
    }

    pub async fn delete_commit(&self, id: CommitId) -> Result<(), ApiError> {
        self.raw("DELETE", &format!("/api/v1/commits/{id}"), None, None)
            .await
            .map(|_| ())
    }

    /// Stars or unstars a commit, and returns it as it is now.
    pub async fn set_star(&self, id: CommitId, starred: bool) -> Result<Commit, ApiError> {
        self.data(
            if starred { "POST" } else { "DELETE" },
            &format!("/api/v1/commits/{id}/star"),
            None,
        )
        .await
    }

    /// Redeems a one-time code from the approval page, which makes the key an
    /// approved device of the user who approved it. Signed with the key,
    /// though it is not approved yet.
    pub async fn approve_device(&self, approval: &DeviceApproval) -> Result<User, ApiError> {
        self.data(
            "POST",
            "/api/v1/devices",
            Some(serde_json::to_vec(approval).map_err(ApiError::Decode)?),
        )
        .await
    }
}

impl Client {
    async fn data<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<T, ApiError> {
        #[derive(serde::Deserialize)]
        struct Resource<T> {
            data: T,
        }
        self.request::<Resource<T>>(method, path, None, body)
            .await
            .map(|r| r.data)
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        cursor: Option<&Cursor>,
        body: Option<Vec<u8>>,
    ) -> Result<T, ApiError> {
        let bytes = self.raw(method, path, cursor, body).await?;
        serde_json::from_slice(&bytes).map_err(ApiError::Decode)
    }

    fn prepare(
        &self,
        method: &str,
        path: &str,
        cursor: Option<&Cursor>,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Request, ApiError> {
        use super::SignedHeaders;
        let mut url = self
            .base
            .as_url()
            .join(path)
            .expect("API path is relative to a valid base");
        if let Some(cursor) = cursor {
            url.query_pairs_mut().append_pair("cursor", cursor.as_str());
        }
        let path_and_query = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let headers =
            self.key
                .sign_request(method, path_and_query, body.as_deref().unwrap_or_default())?;
        let mut request = self
            .http
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).expect("constant method"),
                url,
            )
            .header("Accept", "application/json")
            .header(SignedHeaders::TIMESTAMP, headers.timestamp)
            .header(SignedHeaders::NONCE, headers.nonce)
            .header(SignedHeaders::SIGNATURE, headers.signature);
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        request.build().map_err(ApiError::from)
    }

    async fn raw(
        &self,
        method: &str,
        path: &str,
        cursor: Option<&Cursor>,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ApiError> {
        let request = self.prepare(method, path, cursor, body)?;
        let response = self.http.execute(request).await?;
        let status = response.status();
        let retry_after = (status == reqwest::StatusCode::TOO_MANY_REQUESTS)
            .then(|| {
                response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .map(Duration::from_secs)
            })
            .flatten();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            #[derive(serde::Deserialize)]
            struct ErrorBody {
                code: Option<super::ErrorCode>,
                message: Option<String>,
            }
            let error = serde_json::from_slice::<ErrorBody>(&bytes).ok();
            return Err(ApiError::Status {
                status,
                retry_after,
                code: error.as_ref().and_then(|e| e.code),
                message: error
                    .and_then(|e| e.message)
                    .unwrap_or_else(|| format!("HTTP {status}")),
            });
        }
        Ok(bytes.to_vec())
    }
}

impl Client {
    fn user_path(&self, username: &Username) -> Result<String, ApiError> {
        if matches!(username.as_str(), "" | "." | "..") {
            return Err(ApiError::InvalidUsername);
        }
        let mut url = self
            .base
            .as_url()
            .join("/api/v1/users")
            .expect("constant API path");
        url.path_segments_mut()
            .expect("HTTP URL has path segments")
            .push(username.as_str());
        Ok(url.path().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::SignedHeaders;

    // Signatures cover the final encoded query and exact JSON bytes, not an earlier URL or reserialized body.
    #[test]
    fn prepared_request_signature_matches_wire_bytes() {
        let dir = crate::test_support::TestDir::new();
        let path = dir.0.join("key");
        let key = DeviceKey::generate(&path, "request test").unwrap();
        let public = ssh_key::PrivateKey::from_openssh(std::fs::read_to_string(path).unwrap())
            .unwrap()
            .public_key()
            .clone();
        let client = Client::new(BaseUrl::default(), key).unwrap();
        let body = br#"{"body":"spaces  and\nnewlines"}"#.to_vec();
        let cursor = Cursor::from("a+/= with space".to_owned());
        let request = client
            .prepare("POST", "/api/v1/commits", Some(&cursor), Some(body.clone()))
            .unwrap();
        let headers = request.headers();
        let timestamp = headers[SignedHeaders::TIMESTAMP].to_str().unwrap();
        let nonce = headers[SignedHeaders::NONCE].to_str().unwrap();
        let signature = headers[SignedHeaders::SIGNATURE]
            .as_bytes()
            .chunks(70)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let signature = ssh_key::SshSig::from_pem(format!(
            "-----BEGIN SSH SIGNATURE-----\n{signature}\n-----END SSH SIGNATURE-----\n"
        ))
        .unwrap();
        let path_and_query = &request.url()[url::Position::BeforePath..url::Position::AfterQuery];
        assert_eq!(
            path_and_query,
            "/api/v1/commits?cursor=a%2B%2F%3D+with+space"
        );
        let mut message = format!("{timestamp}\n{nonce}\nPOST\n{path_and_query}\n").into_bytes();
        message.extend_from_slice(request.body().unwrap().as_bytes().unwrap());
        public.verify("devnull-api", &message, &signature).unwrap();
        assert_eq!(request.body().unwrap().as_bytes().unwrap(), body);
        assert_eq!(headers["Accept"], "application/json");
        assert_eq!(headers["Content-Type"], "application/json");
        let get = client.prepare("GET", "/api/v1/me", None, None).unwrap();
        assert!(get.body().is_none());
    }

    // Usernames remain one segment and signatures verify against the exact encoded URL for all three endpoints.
    #[test]
    fn username_paths_are_encoded_before_signing() {
        let dir = crate::test_support::TestDir::new();
        let key_path = dir.0.join("key");
        let key = DeviceKey::generate(&key_path, "profile request test").unwrap();
        let public = ssh_key::PrivateKey::from_openssh(std::fs::read_to_string(key_path).unwrap())
            .unwrap()
            .public_key()
            .clone();
        let client = Client::new(BaseUrl::default(), key).unwrap();
        let username = Username::from("../a/b?x=#%2f\\ name界");
        let path = client.user_path(&username).unwrap();
        assert_eq!(
            path,
            "/api/v1/users/..%2Fa%2Fb%3Fx=%23%252f%5C%20name%E7%95%8C"
        );
        for (method, suffix, cursor) in [
            ("GET", "", None),
            ("GET", "/commits", Some(Cursor::from("+/=".to_owned()))),
            ("POST", "/follow", None),
        ] {
            let request = client
                .prepare(method, &format!("{path}{suffix}"), cursor.as_ref(), None)
                .unwrap();
            assert_eq!(request.url().path(), format!("{path}{suffix}"));
            let headers = request.headers();
            let timestamp = headers[SignedHeaders::TIMESTAMP].to_str().unwrap();
            let nonce = headers[SignedHeaders::NONCE].to_str().unwrap();
            let signature = headers[SignedHeaders::SIGNATURE]
                .as_bytes()
                .chunks(70)
                .map(|line| std::str::from_utf8(line).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let signature = ssh_key::SshSig::from_pem(format!(
                "-----BEGIN SSH SIGNATURE-----\n{signature}\n-----END SSH SIGNATURE-----\n"
            ))
            .unwrap();
            let wire_path = &request.url()[url::Position::BeforePath..url::Position::AfterQuery];
            let message = format!("{timestamp}\n{nonce}\n{method}\n{wire_path}\n");
            public
                .verify("devnull-api", message.as_bytes(), &signature)
                .unwrap();
            assert!(request.body().is_none());
        }
        for username in ["", ".", ".."] {
            assert!(matches!(
                client.user_path(&Username::from(username)),
                Err(ApiError::InvalidUsername)
            ));
        }
    }

    // Only the explicitly permitted loopback hosts may bypass HTTPS.
    #[test]
    fn base_url_restricts_plain_http() {
        for good in [
            "https://dev-null.sh",
            "https://example.test:8443",
            "http://localhost:8000",
            "http://127.0.0.1",
        ] {
            assert!(good.parse::<BaseUrl>().is_ok());
        }
        for bad in [
            "http://example.test",
            "http://localhost.example.test",
            "http://[::1]",
            "ftp://localhost",
            "not a URL",
        ] {
            assert!(bad.parse::<BaseUrl>().is_err());
        }
    }
}
