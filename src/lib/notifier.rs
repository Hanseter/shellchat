use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use futures::future::{ok, Ready};
use log::debug;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Debug, Deserialize, Clone)]
pub struct NotifierConfig {
    pub url: String,
    pub body: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}

pub struct RequestNotifier {
    config: NotifierConfig,
    client: Arc<Client>,
}

impl RequestNotifier {
    pub fn new(config: NotifierConfig, client: Arc<Client>) -> Self {
        RequestNotifier { config, client }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RequestNotifier
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = RequestNotifierMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let header_map = match &self.config.headers {
            Some(headers) => {
                let mut header_map = HeaderMap::new();
                for (key, value) in headers {
                    header_map.insert(
                        HeaderName::from_bytes(key.as_bytes()).unwrap(),
                        HeaderValue::from_str(value).unwrap(),
                    );
                }
                Some(header_map)
            }
            None => None,
        };

        ok(RequestNotifierMiddleware {
            service: Rc::new(service),
            url: self.config.url.clone(),
            header_map,
            body: self.config.body.clone(),
            client: self.client.clone(),
        })
    }
}

pub struct RequestNotifierMiddleware<S> {
    service: Rc<S>,
    url: String,
    header_map: Option<HeaderMap>,
    body: Option<String>,
    client: Arc<Client>,
}

impl<S, B> Service<ServiceRequest> for RequestNotifierMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn futures::Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let fut = self.service.call(req);
        let url = self.url.clone();
        let headers_map = self.header_map.clone();
        let body = self.body.clone();
        let client = self.client.clone();

        actix_rt::spawn(async move {
            let mut request = client.post(url);
            if let Some(headers_map) = headers_map {
                request = request.headers(headers_map);
            }

            if let Some(body) = body {
                request = request.body(body);
            }
            let response = request.send().await;

            match response {
                Ok(res) => {
                    if res.status().is_success() {
                        debug!("Successfully sent log to webhook");
                    } else {
                        debug!("Failed to send log to webhook: {:?}", res.status());
                    }
                }
                Err(e) => {
                    debug!("Error sending log to webhook: {:?}", e);
                }
            }
        });

        Box::pin(async move {
            let res = fut.await?;
            Ok(res)
        })
    }
}
