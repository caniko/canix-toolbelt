//! Complete, read-only forge collection with consumer-owned authenticated transports.
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};

const COMMENT: &str = "id url author{login __typename}body createdAt updatedAt";
const INLINE: &str = "id url author{login __typename}body createdAt updatedAt commit{oid}originalCommit{oid}pullRequestReview{url commit{oid}}";
const CHECKS: &str = "totalCount pageInfo{hasNextPage endCursor}nodes{__typename ... on CheckRun{id name status conclusion detailsUrl startedAt completedAt}... on StatusContext{id context state targetUrl createdAt}}";
const BASIC: &str = "id url title state isDraft updatedAt headRefOid baseRefOid baseRefName baseRef{target{oid}}mergedAt mergeCommit{oid}mergeable mergeStateStatus reviewDecision commits(last:1){nodes{commit{id oid statusCheckRollup{state contexts(first:100){CHECKS}}}}}";

const INLINE_LIMIT: u64 = 2000;
const REQUEST_LIMIT: usize = 128;

struct BoundedGraph<'a, T> {
    inner: &'a mut T,
    remaining: usize,
}
impl<T: GraphQl> GraphQl for BoundedGraph<'_, T> {
    fn query(&mut self, query: &str) -> io::Result<Value> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| invalid("aggregate GitHub request budget exhausted"))?;
        self.inner.query(query)
    }
}

struct BoundedForge<'a, T> {
    inner: &'a mut T,
    remaining: usize,
}
impl<T: ForgeRest> ForgeRest for BoundedForge<'_, T> {
    fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| invalid("aggregate Forgejo request budget exhausted"))?;
        self.inner.get(host, path)
    }
}

/// Host-local authenticated GitHub GraphQL calls. Implementations bound time and bytes.
pub trait GraphQl {
    /// Return the GraphQL `data` object; native errors must be unsuccessful results.
    fn query(&mut self, query: &str) -> io::Result<Value>;
}

/// Host-local authenticated Forgejo GET calls, including native expiry-aware auth.
pub trait ForgeRest {
    /// Return one bounded decoded response, preserving native count metadata.
    /// Missing, duplicated or malformed `x-total-count` must never become a
    /// count inferred from the returned rows.
    fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse>;
}

/// A native Forgejo response and its authoritative pagination metadata.
#[derive(Clone, Debug)]
pub struct ForgeResponse {
    /// Decoded native JSON, without renamed fields or synthesized defaults.
    pub body: Value,
    /// Parsed native `x-total-count`; required on history list endpoints.
    pub total_count: Option<u64>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn quote(value: &str) -> String {
    json!(value).to_string()
}
fn github_revision(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|oid| oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

// Explicit nullable values are native evidence; absent requested fields are not.
fn field<'a>(value: &'a Value, name: &str) -> io::Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| invalid(format!("missing native field {name}")))
}
fn string(value: &Value, name: &str) -> io::Result<()> {
    if field(value, name)?.is_string() {
        Ok(())
    } else {
        Err(invalid(format!("malformed native string {name}")))
    }
}
fn nonempty(value: &Value, name: &str) -> io::Result<()> {
    string(value, name)?;
    if value[name].as_str().is_some_and(|text| !text.is_empty()) {
        Ok(())
    } else {
        Err(invalid(format!("empty native field {name}")))
    }
}
fn boolean(value: &Value, name: &str) -> io::Result<()> {
    if field(value, name)?.is_boolean() {
        Ok(())
    } else {
        Err(invalid(format!("malformed native boolean {name}")))
    }
}
fn enumeration(value: &Value, name: &str, choices: &[&str]) -> io::Result<()> {
    if field(value, name)?
        .as_str()
        .is_some_and(|s| choices.contains(&s))
    {
        Ok(())
    } else {
        Err(invalid(format!("unsupported native enum {name}")))
    }
}
fn timestamp(value: &Value, name: &str) -> io::Result<()> {
    if field(value, name)?
        .as_str()
        .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
    {
        Ok(())
    } else {
        Err(invalid(format!("malformed native timestamp {name}")))
    }
}
fn nullable(
    value: &Value,
    name: &str,
    validate: impl FnOnce(&Value, &str) -> io::Result<()>,
) -> io::Result<()> {
    if field(value, name)?.is_null() {
        Ok(())
    } else {
        validate(value, name)
    }
}
fn github_commit(value: &Value, name: &str) -> io::Result<()> {
    nullable(value, name, |value, name| {
        if github_revision(&value[name]["oid"]) {
            Ok(())
        } else {
            Err(invalid(format!("malformed native commit {name}")))
        }
    })
}
fn github_author(value: &Value) -> io::Result<()> {
    nullable(value, "author", |value, name| {
        nonempty(&value[name], "login")?;
        nonempty(&value[name], "__typename")
    })
}
fn github_comment(value: &Value, inline: bool) -> io::Result<()> {
    nonempty(value, "url")?;
    string(value, "body")?;
    github_author(value)?;
    timestamp(value, "createdAt")?;
    timestamp(value, "updatedAt")?;
    if inline {
        github_commit(value, "commit")?;
        github_commit(value, "originalCommit")?;
        nullable(value, "pullRequestReview", |value, name| {
            nonempty(&value[name], "url")?;
            github_commit(&value[name], "commit")
        })?;
    }
    Ok(())
}
fn github_status(value: &Value) -> io::Result<()> {
    field(value, "baseRef")?;
    enumeration(value, "state", &["OPEN", "CLOSED", "MERGED"])?;
    boolean(value, "isDraft")?;
    nonempty(value, "baseRefName")?;
    enumeration(value, "mergeable", &["MERGEABLE", "CONFLICTING", "UNKNOWN"])?;
    enumeration(
        value,
        "mergeStateStatus",
        &[
            "DIRTY",
            "UNKNOWN",
            "BLOCKED",
            "BEHIND",
            "DRAFT",
            "UNSTABLE",
            "HAS_HOOKS",
            "CLEAN",
        ],
    )?;
    nullable(value, "reviewDecision", |value, name| {
        enumeration(
            value,
            name,
            &["CHANGES_REQUESTED", "APPROVED", "REVIEW_REQUIRED"],
        )
    })?;
    github_commit(value, "mergeCommit")
}

const STATUS_STATES: &[&str] = &["EXPECTED", "ERROR", "FAILURE", "PENDING", "SUCCESS"];

fn github_context(value: &Value) -> io::Result<()> {
    match field(value, "__typename")?.as_str() {
        Some("CheckRun") => {
            string(value, "name")?;
            enumeration(
                value,
                "status",
                &[
                    "REQUESTED",
                    "QUEUED",
                    "IN_PROGRESS",
                    "COMPLETED",
                    "WAITING",
                    "PENDING",
                ],
            )?;
            nullable(value, "conclusion", |value, name| {
                enumeration(
                    value,
                    name,
                    &[
                        "ACTION_REQUIRED",
                        "TIMED_OUT",
                        "CANCELLED",
                        "FAILURE",
                        "SUCCESS",
                        "NEUTRAL",
                        "SKIPPED",
                        "STARTUP_FAILURE",
                        "STALE",
                    ],
                )
            })?;
            nullable(value, "detailsUrl", string)?;
            nullable(value, "startedAt", timestamp)?;
            nullable(value, "completedAt", timestamp)
        }
        Some("StatusContext") => {
            string(value, "context")?;
            enumeration(value, "state", STATUS_STATES)?;
            nullable(value, "targetUrl", string)?;
            timestamp(value, "createdAt")
        }
        _ => Err(invalid("missing or unsupported GitHub check context type")),
    }
}
fn coordinate(url: &str, github: bool) -> io::Result<(&str, &str, &str, u64)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| invalid("PR URL must use HTTPS"))?;
    let parts: Vec<_> = rest.split('/').collect();
    if parts.len() != 5
        || (github && parts[0] != "github.com")
        || (!github && parts[0] == "github.com")
        || parts[..3].iter().any(|part| {
            part.is_empty()
                || matches!(*part, "." | "..")
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        || parts[3] != if github { "pull" } else { "pulls" }
    {
        return Err(invalid("invalid PR coordinate"));
    }
    let number = parts[4]
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .filter(|n| n.to_string() == parts[4])
        .ok_or_else(|| invalid("invalid PR number"))?;
    Ok((parts[0], parts[1], parts[2], number))
}

fn connection(
    transport: &mut impl GraphQl,
    value: &mut Value,
    mut next: impl FnMut(&mut dyn GraphQl, &str) -> io::Result<Value>,
) -> io::Result<()> {
    let mut nodes = value["nodes"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("missing connection nodes"))?;
    if nodes.len() > 100 {
        return Err(invalid("initial connection page exceeds requested bound"));
    }
    let total = value["totalCount"]
        .as_u64()
        .ok_or_else(|| invalid("missing or malformed connection total"))?;
    let mut seen = BTreeSet::new();
    loop {
        let more = value["pageInfo"]["hasNextPage"]
            .as_bool()
            .ok_or_else(|| invalid("missing connection pagination"))?;
        if !more {
            break;
        }
        let cursor = value["pageInfo"]["endCursor"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("missing pagination cursor"))?;
        if seen.len() >= 20 || !seen.insert(cursor.to_owned()) {
            return Err(invalid(
                "pagination bound or repeated cursor; history incomplete",
            ));
        }
        let page = next(transport, cursor)?;
        if page["totalCount"].as_u64() != Some(total) {
            return Err(invalid("connection total changed during collection"));
        }
        let items = page["nodes"]
            .as_array()
            .ok_or_else(|| invalid("missing next-page nodes"))?;
        if items.len() > 100 {
            return Err(invalid("next page exceeds requested bound"));
        }
        nodes.extend(items.iter().cloned());
        value["pageInfo"] = page["pageInfo"].clone();
    }
    if total != nodes.len() as u64 {
        return Err(invalid(
            "connection count does not establish complete history",
        ));
    }
    let mut ids = BTreeSet::new();
    if nodes.iter().any(|node| {
        !node["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && ids.insert(id))
    }) {
        return Err(invalid(
            "missing or repeated connection identity; history incomplete",
        ));
    }
    value["nodes"] = json!(nodes);
    Ok(())
}

fn codex(value: &Value) -> bool {
    value["author"]["__typename"] == "Bot"
        && matches!(
            value["author"]["login"].as_str(),
            Some("chatgpt-codex-connector" | "chatgpt-codex-connector[bot]")
        )
}

fn github_checks(transport: &mut impl GraphQl, pull: &Value) -> io::Result<Value> {
    let commits = pull["commits"]["nodes"]
        .as_array()
        .ok_or_else(|| invalid("missing candidate commit collection"))?;
    if commits.len() != 1 {
        return Err(invalid("expected exactly one checked candidate commit"));
    }
    let commit = &commits[0]["commit"];
    if !commit["oid"].is_string() || commit["oid"] != pull["headRefOid"] {
        return Err(invalid("checked commit does not match the candidate head"));
    }
    let id = commit["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid("missing checked commit identity"))?;
    let mut rollup = field(commit, "statusCheckRollup")?.clone();
    if !rollup.is_null() {
        let contexts = rollup
            .as_object_mut()
            .and_then(|rollup| rollup.get_mut("contexts"))
            .ok_or_else(|| invalid("missing or malformed GitHub check rollup"))?;
        connection(transport, contexts, |transport, cursor| {
            let data = transport.query(&format!("query{{node(id:{}){{... on Commit{{statusCheckRollup{{contexts(first:100,after:{}){{{CHECKS}}}}}}}}}}}",quote(id),quote(cursor)))?;
            Ok(data["node"]["statusCheckRollup"]["contexts"].clone())
        })?;
        for context in contexts["nodes"].as_array().expect("validated connection") {
            github_context(context)?;
        }
        enumeration(&rollup, "state", STATUS_STATES)?;
    }
    Ok(rollup)
}

/// Collect full history, nested review comments and all check contexts. Revalidate
/// the candidate/target/update identity before publishing an observation.
pub fn github(
    transport: &mut impl GraphQl,
    url: &str,
    previous: &Value,
    at: &str,
    now: i64,
) -> io::Result<Value> {
    let mut bounded = BoundedGraph {
        inner: transport,
        remaining: REQUEST_LIMIT,
    };
    let transport = &mut bounded;
    let (_, owner, repo, number) = coordinate(url, true)?;
    let repository = format!("repository(owner:{},name:{})", quote(owner), quote(repo));
    let pull = format!("pullRequest(number:{number})");
    let basic = BASIC.replace("CHECKS", CHECKS);
    let comment = format!("totalCount pageInfo{{hasNextPage endCursor}}nodes{{{COMMENT}}}");
    let reviews = "totalCount pageInfo{hasNextPage endCursor}nodes{id url author{login __typename}state body commit{oid}submittedAt}";
    let thread_comments = format!("totalCount pageInfo{{hasNextPage endCursor}}nodes{{{INLINE}}}");
    let threads = format!(
        "totalCount pageInfo{{hasNextPage endCursor}}nodes{{id isResolved isOutdated path comments(first:100){{{thread_comments}}}}}"
    );
    let query = format!(
        "query{{{repository}{{viewerPermission {pull}{{{basic} comments(first:100){{{comment}}} reviews(first:100){{{reviews}}} reviewThreads(first:100){{{threads}}}}}}}}}"
    );
    let response = transport.query(&query)?;
    let permission = |repository: &Value| {
        nullable(repository, "viewerPermission", |value, name| {
            enumeration(
                value,
                name,
                &[
                    "ADMIN",
                    "MAINTAIN",
                    "WRITE",
                    "TRIAGE_PLUS",
                    "TRIAGE",
                    "READ",
                ],
            )
        })
    };
    permission(&response["repository"])?;
    let mut p = response["repository"]["pullRequest"].clone();
    if !p.is_object()
        || p["id"].as_str().is_none_or(str::is_empty)
        || p["url"].as_str() != Some(url)
        || p["url"].as_str().and_then(|url| coordinate(url, true).ok())
            != Some(("github.com", owner, repo, number))
        || !github_revision(&p["headRefOid"])
        || !github_revision(&p["baseRefOid"])
        || (!p["baseRef"].is_null() && !github_revision(&p["baseRef"]["target"]["oid"]))
        || p["updatedAt"]
            .as_str()
            .is_none_or(|at| chrono::DateTime::parse_from_rfc3339(at).is_err())
    {
        return Err(invalid("missing GitHub candidate identity"));
    }
    github_status(&p)?;
    for (name, selection) in [
        ("comments", comment.as_str()),
        ("reviews", reviews),
        ("reviewThreads", threads.as_str()),
    ] {
        connection(transport, &mut p[name], |transport, cursor| {
            let data = transport.query(&format!(
                "query{{{repository}{{{pull}{{{name}(first:100,after:{}){{{selection}}}}}}}}}",
                quote(cursor)
            ))?;
            Ok(data["repository"]["pullRequest"][name].clone())
        })?;
    }
    for comment in p["comments"]["nodes"]
        .as_array()
        .expect("validated connection")
    {
        github_comment(comment, false)?;
    }
    for review in p["reviews"]["nodes"]
        .as_array()
        .expect("validated connection")
    {
        nonempty(review, "url")?;
        string(review, "body")?;
        github_author(review)?;
        enumeration(
            review,
            "state",
            &[
                "PENDING",
                "COMMENTED",
                "APPROVED",
                "CHANGES_REQUESTED",
                "DISMISSED",
            ],
        )?;
        github_commit(review, "commit")?;
        nullable(review, "submittedAt", timestamp)?;
    }
    let mut inline_total = 0_u64;
    for thread in p["reviewThreads"]["nodes"]
        .as_array()
        .expect("validated connection")
    {
        let count = thread["comments"]["totalCount"]
            .as_u64()
            .ok_or_else(|| invalid("missing or malformed connection total"))?;
        inline_total = inline_total
            .checked_add(count)
            .filter(|total| *total <= INLINE_LIMIT)
            .ok_or_else(|| invalid("aggregate GitHub inline comment bound exceeded"))?;
    }
    let mut inline_ids = BTreeSet::new();
    for thread in p["reviewThreads"]["nodes"]
        .as_array_mut()
        .expect("validated connection")
    {
        let id = thread["id"]
            .as_str()
            .ok_or_else(|| invalid("missing review-thread identity"))?
            .to_owned();
        connection(transport, &mut thread["comments"], |transport, cursor| {
            let data = transport.query(&format!("query{{node(id:{}){{... on PullRequestReviewThread{{comments(first:100,after:{}){{{thread_comments}}}}}}}}}",quote(&id),quote(cursor)))?;
            Ok(data["node"]["comments"].clone())
        })?;
        if thread["comments"]["nodes"]
            .as_array()
            .expect("validated comment connection")
            .iter()
            .any(|comment| {
                !inline_ids.insert(
                    comment["id"]
                        .as_str()
                        .expect("validated comment identity")
                        .to_owned(),
                )
            })
        {
            return Err(invalid(
                "repeated GitHub inline comment identity across threads",
            ));
        }
        boolean(thread, "isResolved")?;
        boolean(thread, "isOutdated")?;
        nonempty(thread, "path")?;
        for comment in thread["comments"]["nodes"]
            .as_array()
            .expect("validated connection")
        {
            github_comment(comment, true)?;
        }
    }
    let rollup = github_checks(transport, &p)?;
    let fresh = transport.query(&format!(
        "query{{{repository}{{viewerPermission {pull}{{{basic}}}}}}}"
    ))?;
    permission(&fresh["repository"])?;
    if fresh["repository"]["viewerPermission"] != response["repository"]["viewerPermission"] {
        return Err(invalid(
            "GitHub repository permission moved during collection",
        ));
    }
    github_status(&fresh["repository"]["pullRequest"])?;
    for key in [
        "id",
        "url",
        "headRefOid",
        "baseRefOid",
        "baseRef",
        "updatedAt",
        "state",
        "mergeable",
        "mergeStateStatus",
        "reviewDecision",
        "isDraft",
        "baseRefName",
        "mergeCommit",
        "commits",
    ] {
        if fresh["repository"]["pullRequest"][key] != p[key] {
            return Err(invalid(
                "candidate, target or feedback moved during collection",
            ));
        }
    }
    if github_checks(transport, &fresh["repository"]["pullRequest"])? != rollup {
        return Err(invalid("GitHub check rollup moved during collection"));
    }
    let mut value = previous
        .as_object()
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    for (key, source) in [
        ("state", "state"),
        ("head", "headRefOid"),
        ("reportedBase", "baseRefOid"),
        ("baseBranch", "baseRefName"),
        ("draft", "isDraft"),
        ("mergeable", "mergeable"),
        ("mergeState", "mergeStateStatus"),
        ("reviewDecision", "reviewDecision"),
        ("mergeCommit", "mergeCommit"),
        ("updatedAt", "updatedAt"),
    ] {
        value[key] = p[source].clone();
    }
    value["base"] = p["baseRef"]["target"]["oid"]
        .as_str()
        .map_or_else(|| p["baseRefOid"].clone(), |s| json!(s));
    value["url"] = json!(url);
    value["permission"] = response["repository"]["viewerPermission"].clone();
    value["capturedAt"] = json!(at);
    value["deepAt"] = json!(now);
    value["checks"] = rollup;
    value["issueComments"] = p["comments"]["nodes"].clone();
    let comments = p["comments"]["nodes"]
        .as_array()
        .expect("validated connection");
    value["comments"] = json!(comments.iter().filter(|c| codex(c)).collect::<Vec<_>>());
    value["requests"] = json!(
        comments
            .iter()
            .filter(|c| !codex(c)
                && c["body"]
                    .as_str()
                    .is_some_and(|body| body.contains("@codex review")
                        || body.contains("@codex security review")))
            .collect::<Vec<_>>()
    );
    value["reviews"] = p["reviews"]["nodes"].clone();
    value["threads"] = p["reviewThreads"]["nodes"].clone();
    value["unresolvedCodex"] = json!(
        p["reviewThreads"]["nodes"]
            .as_array()
            .expect("validated connection")
            .iter()
            .filter(|t| t["isResolved"] == false
                && t["comments"]["nodes"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(codex)))
            .count()
    );
    value["classification"] = super::progress(&value, &Value::Null)["stage"].clone();
    for key in ["lastError", "lastErrorAt"] {
        value.as_object_mut().expect("object").remove(key);
    }
    Ok(value)
}

fn forge_pages(transport: &mut impl ForgeRest, host: &str, path: &str) -> io::Result<Vec<Value>> {
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    let mut declared = None;
    for page in 1..=21 {
        let response = transport.get(host, &format!("{path}?limit=100&page={page}"))?;
        let total = response
            .total_count
            .ok_or_else(|| invalid("missing native forge history total"))?;
        if declared.is_some_and(|previous| previous != total) {
            return Err(invalid("forge history total moved during collection"));
        }
        if total > 2000 {
            return Err(invalid("forge history exceeds 2000 rows"));
        }
        declared = Some(total);
        let rows = response
            .body
            .as_array()
            .ok_or_else(|| invalid("missing forge page"))?;
        if rows.len() != total.saturating_sub(result.len() as u64).min(100) as usize {
            return Err(invalid("incomplete forge history page"));
        }
        if rows.iter().any(|row| {
            !row["id"]
                .as_u64()
                .is_some_and(|id| id > 0 && ids.insert(id))
        }) {
            return Err(invalid("missing or repeated forge history identity"));
        }
        result.extend(rows.iter().cloned());
        if result.len() as u64 == total && (rows.len() < 100 || page == 21) {
            return Ok(result);
        }
    }
    Err(invalid("forge history exceeds pagination bound"))
}

const FORGE_STATUS_STATES: &[&str] = &[
    "pending", "success", "error", "failure", "warning", "skipped",
];

fn forge_user(value: &Value, name: &str) -> io::Result<()> {
    nullable(value, name, |value, name| {
        if value[name]["id"].as_u64().is_none_or(|id| id == 0) {
            return Err(invalid("malformed forge user identity"));
        }
        nonempty(&value[name], "login")
    })
}
fn forge_comment(value: &Value) -> io::Result<()> {
    string(value, "body")?;
    forge_user(value, "user")?;
    timestamp(value, "created_at")?;
    timestamp(value, "updated_at")?;
    string(value, "html_url")
}
fn forge_status(value: &Value) -> io::Result<()> {
    enumeration(value, "state", &["open", "closed"])?;
    for name in ["merged", "mergeable", "draft"] {
        boolean(value, name)?;
    }
    Ok(())
}

fn forge_revision(value: &Value) -> bool {
    value.as_str().is_some_and(|oid| {
        matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn forge_checks(
    transport: &mut impl ForgeRest,
    host: &str,
    stem: &str,
    head: &str,
) -> io::Result<Value> {
    let path = format!("{stem}/commits/{head}/status?limit=100");
    let mut first = transport.get(host, &format!("{path}&page=1"))?.body;
    let total = first["total_count"]
        .as_u64()
        .ok_or_else(|| invalid("missing forge check total"))?;
    if total > 2000 || first["sha"] != head || !first["state"].is_string() {
        return Err(invalid("invalid or oversized forge check identity"));
    }
    enumeration(&first, "state", FORGE_STATUS_STATES)?;
    let mut statuses = first["statuses"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("missing forge check contexts"))?;
    if statuses.len() != total.min(100) as usize {
        return Err(invalid("incomplete initial forge check page"));
    }
    for page in 2..=total.div_ceil(100) {
        let next = transport.get(host, &format!("{path}&page={page}"))?.body;
        if next["sha"] != first["sha"]
            || next["state"] != first["state"]
            || next["total_count"] != first["total_count"]
        {
            return Err(invalid("forge check rollup moved during collection"));
        }
        let rows = next["statuses"]
            .as_array()
            .ok_or_else(|| invalid("missing forge check page"))?;
        if rows.len() != (total - statuses.len() as u64).min(100) as usize {
            return Err(invalid("incomplete forge check page"));
        }
        statuses.extend(rows.iter().cloned());
    }
    let mut ids = BTreeSet::new();
    if statuses.iter().any(|status| {
        !status["id"]
            .as_u64()
            .is_some_and(|id| id > 0 && ids.insert(id))
    }) {
        return Err(invalid("missing or repeated forge check context"));
    }
    for status in &statuses {
        enumeration(status, "status", FORGE_STATUS_STATES)?;
        for name in ["context", "description", "target_url", "url"] {
            string(status, name)?;
        }
        forge_user(status, "creator")?;
        timestamp(status, "created_at")?;
        timestamp(status, "updated_at")?;
    }
    first["statuses"] = json!(statuses);
    Ok(first)
}

/// Collect Forgejo candidate, checks, reviews, issue and nested inline comments.
pub fn forgejo(transport: &mut impl ForgeRest, url: &str, at: &str) -> io::Result<Value> {
    let mut bounded = BoundedForge {
        inner: transport,
        remaining: REQUEST_LIMIT,
    };
    let transport = &mut bounded;
    let (host, owner, repo, number) = coordinate(url, false)?;
    let stem = format!("repos/{owner}/{repo}");
    let path = format!("{stem}/pulls/{number}");
    let p = transport.get(host, &path)?.body;
    if p["number"].as_u64() != Some(number)
        || p["html_url"]
            .as_str()
            .and_then(|url| coordinate(url, false).ok())
            != Some((host, owner, repo, number))
        || p["html_url"].as_str() != Some(url)
    {
        return Err(invalid(
            "Forgejo pull identity differs from requested coordinate",
        ));
    }
    let updated = p["updated_at"]
        .as_str()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .ok_or_else(|| invalid("missing or malformed forge update identity"))?;
    let base = p["base"]["sha"]
        .as_str()
        .filter(|_| forge_revision(&p["base"]["sha"]))
        .ok_or_else(|| invalid("missing or malformed forge base identity"))?;
    let branch = p["base"]["ref"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("missing forge base identity"))?;
    let head = p["head"]["sha"]
        .as_str()
        .filter(|_| forge_revision(&p["head"]["sha"]))
        .ok_or_else(|| invalid("missing or malformed forge candidate identity"))?;
    forge_status(&p)?;
    let reviews = forge_pages(transport, host, &format!("{path}/reviews"))?;
    let comments = forge_pages(transport, host, &format!("{stem}/issues/{number}/comments"))?;
    let mut inline_total = 0_u64;
    for review in &reviews {
        let count = review["comments_count"]
            .as_u64()
            .filter(|count| *count <= INLINE_LIMIT)
            .ok_or_else(|| invalid("missing, malformed or oversized review comment count"))?;
        inline_total = inline_total
            .checked_add(count)
            .filter(|total| *total <= INLINE_LIMIT)
            .ok_or_else(|| invalid("aggregate Forgejo inline comment bound exceeded"))?;
    }
    for review in &reviews {
        string(review, "body")?;
        forge_user(review, "user")?;
        enumeration(
            review,
            "state",
            &[
                "APPROVED",
                "PENDING",
                "COMMENT",
                "REQUEST_CHANGES",
                "REQUEST_REVIEW",
                "",
            ],
        )?;
        timestamp(review, "submitted_at")?;
        timestamp(review, "updated_at")?;
        string(review, "commit_id")?;
        for name in ["html_url", "pull_request_url"] {
            string(review, name)?;
        }
        for name in ["stale", "official", "dismissed"] {
            boolean(review, name)?;
        }
    }
    for comment in &comments {
        forge_comment(comment)?;
    }
    let mut inline = Vec::new();
    let mut inline_ids = BTreeSet::new();
    for review in &reviews {
        let count = review["comments_count"]
            .as_u64()
            .filter(|count| *count <= 2000)
            .ok_or_else(|| invalid("missing, malformed or oversized review comment count"))?;
        if count > 0 {
            let id = review["id"]
                .as_u64()
                .ok_or_else(|| invalid("missing review identity"))?;
            let comments = forge_pages(transport, host, &format!("{path}/reviews/{id}/comments"))?;
            if comments.len() as u64 != count {
                return Err(invalid(
                    "forge review comment count does not establish complete history",
                ));
            }
            for comment in &comments {
                forge_comment(comment)?;
                for name in [
                    "path",
                    "commit_id",
                    "original_commit_id",
                    "pull_request_url",
                ] {
                    string(comment, name)?;
                }
                if comment["pull_request_review_id"].as_u64() != Some(id) {
                    return Err(invalid("forge inline comment differs from parent review"));
                }
            }
            if comments.iter().any(|comment| {
                !inline_ids.insert(comment["id"].as_u64().expect("validated comment ID"))
            }) {
                return Err(invalid(
                    "repeated forge inline comment identity across reviews",
                ));
            }
            inline.extend(comments);
        }
    }
    let checks = forge_checks(transport, host, &stem, head)?;
    let fresh = transport.get(host, &path)?.body;
    if fresh["updated_at"].as_str() != Some(updated) {
        return Err(invalid("forge update identity moved during collection"));
    }
    for key in [
        "number",
        "html_url",
        "head",
        "base",
        "state",
        "merged",
        "mergeable",
        "draft",
        "updated_at",
    ] {
        if fresh[key] != p[key] {
            return Err(invalid(
                "forge candidate or feedback moved during collection",
            ));
        }
    }
    if forge_checks(transport, host, &stem, head)? != checks {
        return Err(invalid("forge check rollup moved after history collection"));
    }
    let state = if p["merged"] == true {
        "MERGED".to_owned()
    } else {
        p["state"]
            .as_str()
            .ok_or_else(|| invalid("missing forge state"))?
            .to_ascii_uppercase()
    };
    Ok(
        json!({"url":url,"state":state,"head":head,"base":base,"baseBranch":branch,"mergeable":p["mergeable"],"draft":p["draft"],"comments":comments,"reviews":reviews,"inlineComments":inline,"checks":checks,"capturedAt":at}),
    )
}
