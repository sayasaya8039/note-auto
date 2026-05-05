# Phase 3 quality report

> Investigator: code-reviewer (silent worker)
> Date: 2026-05-06
> Scope: send_with_retry gaps / concurrency / SSRF / data integrity

---

## CRITICAL (0)

No hardcoded credentials, SQL injection, or auth bypasses found.
No API key values appear in tracing calls (only presence is checked, not values).

---

## HIGH (1)

### H1: src/ai/pollo.rs:119-123 -- Polling GET completely unretried

poll_until_done() uses bare .send().await? per iteration.
non-2xx uses continue (correct). But when send() itself returns Err
(connect error / timeout), ? aborts the entire polling task via propagation.
Polling runs 60 iterations x 5s = 5 minutes. Any network blip kills the task
even when the Pollo server-side job completed successfully.

Impact:
- generate_images() converts Err to None; article created but ALL images missing.
  articles.json shows inline_image_paths: []
- 5 minutes of Pollo API billed, images not received.

Fix (pollo.rs:119) -- use match+continue instead of ?:

    // BAD: one Err aborts polling entirely
    let resp = self.http.get(url).send().await?;

    // GOOD: Err -> continue, polling resumes
    let resp = match crate::util::send_with_retry(
        || self.http.get(url).header("x-api-key", ...).send(),
        2, "pollo_poll",
    ).await {
        Ok(r) => r,
        Err(e) => { tracing::warn!(...); continue; }
    };

Note: use match+continue not ? -- converts Err to skip-iteration not task abort.
Also: send_with_retry (util.rs:142) retries only is_connect()/is_timeout() errors;
match+continue catches ALL reqwest error kinds -- which is why match+continue is
strictly better than adding send_with_retry alone to this path.

---

## MEDIUM (3)

### M1: src/publish/x_post.rs:56,94 -- X API POST unretried

Both announce() and post_text() use bare .send().await?.
X API v2 returns 429 frequently on Free/Basic tiers.
A single 429 immediately Errs and adds to publish_one errors vec.

Impact:
- Note-published articles have X announcement permanently fail.
- Slack final report shows x_status: error, confusing operators.

Fix: wrap with send_with_retry(max_retries=3, label="x_api").

---

### M2: Multiple secondary image download paths unretried

| File                    | Line  | Path                                  |
|-------------------------|-------|---------------------------------------|
| src/ai/pollo.rs         | 86-93 | Final PNG download after poll success |
| src/ai/openai.rs        | 75    | Download when OpenAI returns URL       |
| src/ai/nvidia.rs        | 141   | fetch_url fallback                    |

All use bare .send().await?. For Pollo: after 5 min polling succeeds, final
PNG download can fail on one network error, wasting all polling work.

Fix: add fetch_bytes_with_retry helper to util.rs:
    pub async fn fetch_bytes_with_retry(http, url, label) -> anyhow::Result<Vec<u8>>
    { Ok(send_with_retry(|| http.get(url).send(), 3, label).await?
         .error_for_status()?.bytes().await?.to_vec()) }

---

### M3: src/writer/mod.rs:304 -- Insufficient URL validation in download_source_images

Current check: .filter(|u| u.starts_with("http"))  // prefix only, insufficient

URLs from konbini/hyakkin scrapers (retail CDNs) but check does not block:
- http://localhost:8080/internal-api
- http://192.168.1.1/admin
- http://169.254.169.254/latest/meta-data/ (cloud metadata endpoint)

Risk: LOW on developer machine, HIGH on AWS/GCP/Azure VM.

Fix A -- HTTPS + private IP block:
    fn is_safe_image_url(u: &str) -> bool {
        if !u.starts_with("https://") { return false; }
        // parse IP, reject loopback/private/link-local
        true  // non-IP hostnames pass through
    }

Fix B -- domain allowlist for known retail CDNs (stricter).

---

## LOW (3)

### L1: src/publish/slack.rs:26 -- post_summary unretried, error propagates

Slack 503 causes notify_summary to return Err, logging cycle as failed
even when publish succeeded. post_progress (best-effort) correctly ignores
errors; post_summary inconsistently propagates them.

Fix: apply send_with_retry + tracing::warn on failure.

---

### L2: src/writer/mod.rs:482 -- Slug collision under buffer_unordered

sanitize_slug uses slug::slugify + take(60). Pure Japanese falls back to
article-{index} (unique). English-heavy titles with identical first 60 chars
after slugification produce the same slug. Concurrent buffer_unordered tasks
race-write to the same .md and image files.

Impact: articles.json has duplicate slugs, one actual file on disk.
Silent loss of one article. Triggered when AI returns similar English titles.

Fix: always append index to guarantee uniqueness:
    fn sanitize_slug(raw: &str, idx: usize) -> String {
        let base = slug::slugify(raw).chars().take(55).collect::<String>();
        let base = if base.is_empty() { "article".into() } else { base };
        format!("{base}-{}", idx + 1)  // always unique
    }

Breaking-change note: appending -{idx+1} to every slug changes filename format
even for non-empty slugs (e.g. 'ai-tools-2026' becomes 'ai-tools-2026-3').
Downstream consumers -- Playwright sidecar (reads .md by slug path),
articles.json merge step, and manual review tooling -- must be updated
before deploying this fix to production.

---

### L3: src/trends/x_grok.rs -- call_chat_fallback unretried

call_responses_api has PER_MODEL_RETRIES=2 retry loop. But the final chat
fallback uses bare .send().await? with no retry. Network error aborts
fetch_x_trends entirely.

Fix: wrap with send_with_retry(max_retries=2, label="grok_chat_fallback").

---

## Items confirmed clean

| Item                                        | Result                              |
|---------------------------------------------|-------------------------------------|
| API key / bearer values in tracing calls    | CLEAN -- only presence checked      |
| history.rs double-load (after M1 fix)       | CLEAN -- single mut history instance|
| writer manifest write race                  | CLEAN -- written after collect().await|
| send_with_retry on all 6 AI client POSTs    | CLEAN -- all main POSTs covered     |
| LockGuard RAII drop path                    | CLEAN -- file removed reliably      |
| playwright_script path traversal            | LOW RISK -- config origin, exists() |

---

## Priority summary

| ID | Severity | File                              | Issue                              |
|----|----------|-----------------------------------|------------------------------------|
| H1 | HIGH     | ai/pollo.rs:119                   | Polling GET unretried -> image loss|
| M1 | MEDIUM   | publish/x_post.rs:56,94           | X API unretried -> 429 perm fail   |
| M2 | MEDIUM   | ai/pollo.rs:86/openai:75/nvidia:141| Secondary image DL unretried      |
| M3 | MEDIUM   | writer/mod.rs:304                 | SSRF: URL check only http prefix   |
| L1 | LOW      | publish/slack.rs:26               | post_summary error propagates      |
| L2 | LOW      | writer/mod.rs:482                 | Slug collision under buffer_unordered|
| L3 | LOW      | trends/x_grok.rs                  | chat_fallback unretried            |

## Review Summary

| Severity | Count | Status |
|----------|-------|--------|
| CRITICAL | 0     | pass   |
| HIGH     | 1     | warn   |
| MEDIUM   | 3     | warn   |
| LOW      | 3     | note   |

Verdict: WARNING
H1 (Pollo polling GET unretried) likely causing silent image loss in production.
M3 (SSRF) upgrades to HIGH if deployed on cloud VMs.
H1 + M1 recommended for v0.8.2 fix sprint.
