// Type-state builder: required fields are encoded in the generic type
// parameters via marker types. Calling `.build()` before every required
// field is set is a compile error, not a runtime panic — the
// `RequestBuilder<Url, Method>` impl block carries the values, and
// `RequestBuilder<Unset, Unset>` does not.

pub struct Unset;
pub struct Url(String);
pub struct Method(&'static str);

pub struct RequestBuilder<U, M> {
    url: U,
    method: M,
}

impl RequestBuilder<Unset, Unset> {
    pub fn new() -> Self {
        Self {
            url: Unset,
            method: Unset,
        }
    }
}

impl Default for RequestBuilder<Unset, Unset> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> RequestBuilder<Unset, M> {
    pub fn url(self, url: impl Into<String>) -> RequestBuilder<Url, M> {
        RequestBuilder {
            url: Url(url.into()),
            method: self.method,
        }
    }
}

impl<U> RequestBuilder<U, Unset> {
    pub fn method(self, method: &'static str) -> RequestBuilder<U, Method> {
        RequestBuilder {
            url: self.url,
            method: Method(method),
        }
    }
}

// `build` only exists when both fields are typed (i.e. set). Calling it on
// any other state — even after partial setup — is a compile error.
impl RequestBuilder<Url, Method> {
    pub fn build(self) -> Request {
        Request {
            url: self.url.0,
            method: self.method.0,
        }
    }
}

pub struct Request {
    pub url: String,
    pub method: &'static str,
}
