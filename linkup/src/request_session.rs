use crate::{HeaderMap, extract_tracestate_session, first_subdomain, headers::HeaderName};

pub fn session_names_from_request<'a>(
    url: &'a str,
    headers: &'a HeaderMap,
) -> impl Iterator<Item = String> + 'a {
    let header_candidates = [
        HeaderName::ForwardedHost,
        HeaderName::Referer,
        HeaderName::Origin,
    ]
    .into_iter()
    .filter_map(move |header| headers.get(header).map(first_subdomain));

    let tracestate_candidate = std::iter::once_with(move || {
        headers
            .get(HeaderName::TraceState)
            .map(extract_tracestate_session)
    })
    .flatten();

    std::iter::once_with(move || first_subdomain(url))
        .chain(header_candidates)
        .chain(tracestate_candidate)
}
