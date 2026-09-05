use std::error::Error;
use std::sync::{LazyLock, OnceLock};
use std::{borrow::Cow, mem, pin::Pin, task::Poll, time::Duration};

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use futures::{FutureExt as _, TryStreamExt as _};
use gpui::http_client::{self, RedirectPolicy, Url, http};
use regex::Regex;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    redirect,
};

mod http_client_tls;

const DEFAULT_CAPACITY: usize = 4096;
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static REDACT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"key=[^&]+").unwrap());

pub struct ReqwestClient {
    client: reqwest::Client,
    proxy: Option<Url>,
    user_agent: Option<HeaderValue>,
    handle: tokio::runtime::Handle,
}

impl ReqwestClient {
    fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
    }

    pub fn new() -> anyhow::Result<Self> {
        let client = Self::builder().build()?;
        Self::from_client(client)
    }

    pub fn user_agent(agent: &str) -> anyhow::Result<Self> {
        let mut map = HeaderMap::new();
        map.insert(http::header::USER_AGENT, HeaderValue::from_str(agent)?);
        let client = Self::builder().default_headers(map).build()?;
        Self::from_client(client)
    }

    pub fn proxy_and_user_agent(proxy: Option<Url>, user_agent: &str) -> anyhow::Result<Self> {
        let user_agent = HeaderValue::from_str(user_agent)?;

        let mut map = HeaderMap::new();
        map.insert(http::header::USER_AGENT, user_agent.clone());
        let mut client = Self::builder().default_headers(map);
        if let Some(proxy_url) = proxy.as_ref() {
            let proxy_config = reqwest::Proxy::all(proxy_url.clone()).map_err(|err| {
                anyhow!(
                    "failed to configure proxy '{}': {}",
                    proxy_url,
                    err.source().unwrap_or(&err as &_)
                )
            })?;

            // Respect NO_PROXY env var
            client = client.proxy(proxy_config.no_proxy(reqwest::NoProxy::from_env()));
        }

        let client = client
            .use_preconfigured_tls(http_client_tls::tls_config())
            .build()?;
        let mut client = Self::from_client(client)?;
        client.proxy = proxy;
        client.user_agent = Some(user_agent);
        Ok(client)
    }

    fn from_client(client: reqwest::Client) -> anyhow::Result<Self> {
        let handle = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                log::debug!("no tokio runtime found, creating one for Reqwest...");
                if let Some(runtime) = RUNTIME.get() {
                    runtime.handle().clone()
                } else {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        // Since we now have two executors, let's try to keep our footprint small
                        .worker_threads(1)
                        .enable_all()
                        .build()?;
                    let handle = runtime.handle().clone();

                    if RUNTIME.set(runtime).is_err() {
                        return Ok(Self {
                            client,
                            handle: RUNTIME
                                .get()
                                .map(|runtime| runtime.handle().clone())
                                .unwrap_or(handle),
                            proxy: None,
                            user_agent: None,
                        });
                    }

                    handle
                }
            }
        };

        Ok(Self {
            client,
            handle,
            proxy: None,
            user_agent: None,
        })
    }
}

// This struct is essentially a re-implementation of
// https://docs.rs/tokio-util/0.7.12/tokio_util/io/struct.ReaderStream.html
// except outside of Tokio's aegis
struct StreamReader {
    reader: Option<Pin<Box<dyn futures::AsyncRead + Send + Sync>>>,
    buf: BytesMut,
    capacity: usize,
}

impl StreamReader {
    fn new(reader: Pin<Box<dyn futures::AsyncRead + Send + Sync>>) -> Self {
        Self {
            reader: Some(reader),
            buf: BytesMut::new(),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl futures::Stream for StreamReader {
    type Item = std::io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut();

        let mut reader = match this.reader.take() {
            Some(r) => r,
            None => return Poll::Ready(None),
        };

        if this.buf.capacity() == 0 {
            let capacity = this.capacity;
            this.buf.reserve(capacity);
        }

        match poll_read_buf(&mut reader, cx, &mut this.buf) {
            Poll::Pending => {
                self.reader = Some(reader);
                Poll::Pending
            }
            Poll::Ready(Err(err)) => {
                self.reader = None;

                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Ok(0)) => {
                self.reader = None;
                Poll::Ready(None)
            }
            Poll::Ready(Ok(_)) => {
                let chunk = this.buf.split();
                self.reader = Some(reader);
                Poll::Ready(Some(Ok(chunk.freeze())))
            }
        }
    }
}

/// Append bytes from a futures reader to an initialized buffer.
///
/// A pending read or error leaves the buffer's existing contents unchanged.
pub fn poll_read_buf(
    io: &mut Pin<Box<dyn futures::AsyncRead + Send + Sync>>,
    cx: &mut std::task::Context<'_>,
    buf: &mut BytesMut,
) -> Poll<std::io::Result<usize>> {
    let len = buf.len();
    buf.reserve(1);
    buf.resize(buf.capacity(), 0);
    let result = io.as_mut().poll_read(cx, &mut buf[len..]);
    match result {
        Poll::Ready(Ok(n)) => {
            assert!(n <= buf.len() - len, "reader exceeded buffer capacity");
            buf.truncate(len + n);
        }
        Poll::Pending | Poll::Ready(Err(_)) => buf.truncate(len),
    }
    result
}

fn redact_error(mut error: reqwest::Error) -> reqwest::Error {
    if let Some(url) = error.url_mut()
        && let Some(query) = url.query()
        && let Cow::Owned(redacted) = REDACT_REGEX.replace_all(query, "key=REDACTED")
    {
        url.set_query(Some(redacted.as_str()));
    }
    error
}

impl http_client::HttpClient for ReqwestClient {
    fn proxy(&self) -> Option<&Url> {
        self.proxy.as_ref()
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        self.user_agent.as_ref()
    }

    fn send(
        &self,
        req: http::Request<http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<
        'static,
        anyhow::Result<http_client::Response<http_client::AsyncBody>>,
    > {
        let (parts, body) = req.into_parts();

        let mut request = self.client.request(parts.method, parts.uri.to_string());
        request = request.headers(parts.headers);
        if let Some(redirect_policy) = parts.extensions.get::<RedirectPolicy>() {
            request = request.redirect_policy(match redirect_policy {
                RedirectPolicy::NoFollow => redirect::Policy::none(),
                RedirectPolicy::FollowLimit(limit) => redirect::Policy::limited(*limit as usize),
                RedirectPolicy::FollowAll => redirect::Policy::limited(100),
            });
        }
        let request = request.body(match body.0 {
            http_client::Inner::Empty => reqwest::Body::default(),
            http_client::Inner::Bytes(cursor) => cursor.into_inner().into(),
            http_client::Inner::AsyncReader(stream) => {
                reqwest::Body::wrap_stream(StreamReader::new(stream))
            }
        });

        let handle = self.handle.clone();
        async move {
            let mut response = handle
                .spawn(async { request.send().await })
                .await?
                .map_err(redact_error)?;

            let headers = mem::take(response.headers_mut());
            let mut builder = http::Response::builder()
                .status(response.status().as_u16())
                .version(response.version());
            *builder.headers_mut().unwrap() = headers;

            let bytes = response
                .bytes_stream()
                .map_err(futures::io::Error::other)
                .into_async_read();
            let body = http_client::AsyncBody::from_reader(bytes);

            builder.body(body).map_err(|e| anyhow!(e))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, io, pin::Pin, task::Context};

    use futures::{AsyncRead, Stream, task::noop_waker_ref};
    use gpui::http_client::{HttpClient, Url};

    use super::{BytesMut, Poll, StreamReader, poll_read_buf};
    use crate::ReqwestClient;

    enum ReadStep {
        Pending,
        Data(&'static [u8]),
        Error,
    }

    struct ScriptedReader(VecDeque<ReadStep>);

    impl AsyncRead for ScriptedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            match self.0.pop_front() {
                Some(ReadStep::Pending) => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Some(ReadStep::Data(bytes)) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    if n < bytes.len() {
                        self.0.push_front(ReadStep::Data(&bytes[n..]));
                    }
                    Poll::Ready(Ok(n))
                }
                Some(ReadStep::Error) => Poll::Ready(Err(io::Error::other("read failed"))),
                None => Poll::Ready(Ok(0)),
            }
        }
    }

    #[test]
    fn stream_reader_ready_data_and_eof() {
        let mut stream = StreamReader::new(Box::pin(futures::io::Cursor::new(b"ready")));
        let mut cx = Context::from_waker(noop_waker_ref());
        let Poll::Ready(Some(Ok(bytes))) = Pin::new(&mut stream).poll_next(&mut cx) else {
            panic!("ready reader must yield data");
        };
        assert_eq!(&bytes[..], b"ready");
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }

    #[test]
    fn stream_reader_preserves_reader_across_pending() {
        let reader = ScriptedReader(VecDeque::from([
            ReadStep::Pending,
            ReadStep::Data(b"first"),
            ReadStep::Pending,
            ReadStep::Data(b"second"),
        ]));
        let mut stream = StreamReader::new(Box::pin(reader));
        let mut cx = Context::from_waker(noop_waker_ref());
        for expected in [b"first".as_slice(), b"second".as_slice()] {
            assert!(Pin::new(&mut stream).poll_next(&mut cx).is_pending());
            let Poll::Ready(Some(Ok(bytes))) = Pin::new(&mut stream).poll_next(&mut cx) else {
                panic!("reader must yield data after pending");
            };
            assert_eq!(&bytes[..], expected);
        }
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }

    #[test]
    fn stream_reader_preserves_all_bytes_across_chunks() {
        let input: Vec<u8> = (0..super::DEFAULT_CAPACITY * 3 + 17)
            .map(|index| (index % 251) as u8)
            .collect();
        let mut stream = StreamReader::new(Box::pin(futures::io::Cursor::new(input.clone())));
        let mut cx = Context::from_waker(noop_waker_ref());
        let mut output = Vec::new();
        let mut chunks = 0;
        loop {
            match Pin::new(&mut stream).poll_next(&mut cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    assert!(!bytes.is_empty());
                    output.extend_from_slice(&bytes);
                    chunks += 1;
                }
                Poll::Ready(None) => break,
                other => panic!("unexpected read result: {other:?}"),
            }
        }
        assert!(chunks > 1);
        assert_eq!(output, input);
    }

    #[test]
    fn stream_reader_error_terminates_stream() {
        let reader = ScriptedReader(VecDeque::from([
            ReadStep::Error,
            ReadStep::Data(b"unreachable"),
        ]));
        let mut stream = StreamReader::new(Box::pin(reader));
        let mut cx = Context::from_waker(noop_waker_ref());
        let Poll::Ready(Some(Err(error))) = Pin::new(&mut stream).poll_next(&mut cx) else {
            panic!("reader error must reach the caller");
        };
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "read failed");
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }

    #[test]
    fn poll_read_buf_preserves_prefix_and_ignores_uncommitted_bytes() {
        let mut reader: Pin<Box<dyn AsyncRead + Send + Sync>> =
            Box::pin(ScriptedReader(VecDeque::from([
                ReadStep::Pending,
                ReadStep::Data(b"data"),
                ReadStep::Error,
            ])));
        let mut buf = BytesMut::with_capacity(32);
        buf.extend_from_slice(b"prefix:");
        let mut cx = Context::from_waker(noop_waker_ref());
        assert!(poll_read_buf(&mut reader, &mut cx, &mut buf).is_pending());
        assert_eq!(&buf[..], b"prefix:");
        assert!(matches!(
            poll_read_buf(&mut reader, &mut cx, &mut buf),
            Poll::Ready(Ok(4))
        ));
        assert_eq!(&buf[..], b"prefix:data");
        assert!(matches!(
            poll_read_buf(&mut reader, &mut cx, &mut buf),
            Poll::Ready(Err(_))
        ));
        assert_eq!(&buf[..], b"prefix:data");
    }

    #[test]
    fn test_proxy_uri() {
        let client = ReqwestClient::new().unwrap();
        assert_eq!(client.proxy(), None);

        let proxy = Url::parse("http://localhost:10809").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));

        let proxy = Url::parse("https://localhost:10809").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));

        let proxy = Url::parse("socks4://localhost:10808").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));

        let proxy = Url::parse("socks4a://localhost:10808").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));

        let proxy = Url::parse("socks5://localhost:10808").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));

        let proxy = Url::parse("socks5h://localhost:10808").unwrap();
        let client = ReqwestClient::proxy_and_user_agent(Some(proxy.clone()), "test").unwrap();
        assert_eq!(client.proxy(), Some(&proxy));
    }

    #[test]
    fn test_invalid_proxy_uri() {
        let proxy = Url::parse("socks://127.0.0.1:20170").unwrap();
        let error = match ReqwestClient::proxy_and_user_agent(Some(proxy), "test") {
            Ok(_) => panic!("invalid proxy configuration should fail explicitly"),
            Err(err) => err,
        };
        assert!(
            error.to_string().contains("failed to configure proxy"),
            "invalid proxy configuration should fail explicitly"
        );
    }
}
