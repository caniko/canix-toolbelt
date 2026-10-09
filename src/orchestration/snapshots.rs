//! Complete, read-only forge collection with consumer-owned authenticated transports.
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};

const COMMENT: &str = "id url author{login __typename}body createdAt updatedAt";
const INLINE: &str = "id url author{login __typename}body createdAt updatedAt commit{oid}originalCommit{oid}pullRequestReview{url commit{oid}}";
const CHECKS: &str = "totalCount pageInfo{hasNextPage endCursor}nodes{__typename ... on CheckRun{id name status conclusion detailsUrl startedAt completedAt}... on StatusContext{id context state targetUrl createdAt}}";
const BASIC: &str = "id url title state isDraft updatedAt headRefOid baseRefOid baseRefName baseRef{target{oid}}mergedAt mergeCommit{oid}mergeable mergeStateStatus reviewDecision commits(last:1){nodes{commit{id oid statusCheckRollup{state contexts(first:100){CHECKS}}}}}";

/// Host-local authenticated GitHub GraphQL calls. Implementations bound time and bytes.
pub trait GraphQl {
    /// Return the GraphQL `data` object; native errors must be unsuccessful results.
    fn query(&mut self, query: &str) -> io::Result<Value>;
}

/// Host-local authenticated Forgejo GET calls, including native expiry-aware auth.
pub trait ForgeRest {
    /// Return one bounded decoded response from this declared forge.
    fn get(&mut self, host: &str, path: &str) -> io::Result<Value>;
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn quote(value: &str) -> String {
    json!(value).to_string()
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
        || !matches!(parts[3], "pull" | "pulls")
    {
        return Err(invalid("invalid PR coordinate"));
    }
    let number = parts[4]
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
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
    let mut rollup = commit["statusCheckRollup"].clone();
    if !rollup.is_null() {
        connection(transport, &mut rollup["contexts"], |transport, cursor| {
            let data = transport.query(&format!("query{{node(id:{}){{... on Commit{{statusCheckRollup{{contexts(first:100,after:{}){{{CHECKS}}}}}}}}}}}",quote(id),quote(cursor)))?;
            Ok(data["node"]["statusCheckRollup"]["contexts"].clone())
        })?;
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
    let mut p = response["repository"]["pullRequest"].clone();
    if !p.is_object()
        || !p["headRefOid"].is_string()
        || !p["baseRefOid"].is_string()
        || !p["updatedAt"].is_string()
    {
        return Err(invalid("missing GitHub candidate identity"));
    }
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
    }
    let rollup = github_checks(transport, &p)?;
    let fresh = transport.query(&format!("query{{{repository}{{{pull}{{{basic}}}}}}}"))?;
    for key in [
        "headRefOid",
        "baseRefOid",
        "baseRef",
        "updatedAt",
        "state",
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
    for page in 1..=20 {
        let value = transport.get(host, &format!("{path}?limit=100&page={page}"))?;
        let rows = value
            .as_array()
            .ok_or_else(|| invalid("missing forge page"))?;
        if rows.len() > 100 {
            return Err(invalid("forge page exceeds bound"));
        }
        if rows.iter().any(|row| {
            !row["id"]
                .as_u64()
                .is_some_and(|id| id > 0 && ids.insert(id))
        }) {
            return Err(invalid("missing or repeated forge history identity"));
        }
        result.extend(rows.iter().cloned());
        if rows.len() < 100 {
            return Ok(result);
        }
    }
    Err(invalid("forge pagination exceeds 2000 rows"))
}

fn forge_checks(
    transport: &mut impl ForgeRest,
    host: &str,
    stem: &str,
    head: &str,
) -> io::Result<Value> {
    let path = format!("{stem}/commits/{head}/status?limit=100");
    let mut first = transport.get(host, &format!("{path}&page=1"))?;
    let total = first["total_count"]
        .as_u64()
        .ok_or_else(|| invalid("missing forge check total"))?;
    if total > 2000 || first["sha"] != head || !first["state"].is_string() {
        return Err(invalid("invalid or oversized forge check identity"));
    }
    let mut statuses = first["statuses"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("missing forge check contexts"))?;
    if statuses.len() != total.min(100) as usize {
        return Err(invalid("incomplete initial forge check page"));
    }
    for page in 2..=total.div_ceil(100) {
        let next = transport.get(host, &format!("{path}&page={page}"))?;
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
    if statuses
        .iter()
        .any(|status| !status["id"].as_u64().is_some_and(|id| ids.insert(id)))
    {
        return Err(invalid("missing or repeated forge check context"));
    }
    first["statuses"] = json!(statuses);
    Ok(first)
}

/// Collect Forgejo candidate, checks, reviews, issue and nested inline comments.
pub fn forgejo(transport: &mut impl ForgeRest, url: &str, at: &str) -> io::Result<Value> {
    let (host, owner, repo, number) = coordinate(url, false)?;
    let stem = format!("repos/{owner}/{repo}");
    let path = format!("{stem}/pulls/{number}");
    let p = transport.get(host, &path)?;
    let updated = p["updated_at"]
        .as_str()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .ok_or_else(|| invalid("missing or malformed forge update identity"))?;
    let head = p["head"]["sha"]
        .as_str()
        .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit()) && !s.is_empty())
        .ok_or_else(|| invalid("missing forge candidate identity"))?;
    let reviews = forge_pages(transport, host, &format!("{path}/reviews"))?;
    let comments = forge_pages(transport, host, &format!("{stem}/issues/{number}/comments"))?;
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
    let fresh = transport.get(host, &path)?;
    if fresh["updated_at"].as_str() != Some(updated) {
        return Err(invalid("forge update identity moved during collection"));
    }
    for key in ["head", "base", "state", "merged", "updated_at"] {
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
        json!({"url":url,"state":state,"head":head,"base":p["base"]["sha"],"baseBranch":p["base"]["ref"],"mergeable":p["mergeable"],"draft":p["draft"],"comments":comments,"reviews":reviews,"inlineComments":inline,"checks":checks,"capturedAt":at}),
    )
}
