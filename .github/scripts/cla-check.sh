#!/usr/bin/env bash
# Contributor Licence Agreement check (ADR 0028, CLA.md), run by
# .github/workflows/cla.yml. It replaces contributor-assistant/github-action,
# whose repository is archived and which read only the first 100 commits of
# a pull request (PR #48 review).
#
# On a pull request event it lists the author of every commit (all pages),
# compares them with the signatures in signatures/version1/cla.json on the
# `cla-signatures` branch, sets the `CLA` commit status on the head, and keeps
# one comment on the pull request up to date. On a pull request comment that
# is exactly the signing text, it records the commenter's signature first.
#
# Nothing from the pull request is checked out or run: everything comes from
# the GitHub API, and every value from the event arrives in an environment
# variable and is only compared or passed to jq as data.
#
# Environment: GH_TOKEN, REPO (owner/name), PR (number), EVENT
# (pull_request_target or issue_comment), and for comments COMMENT_BODY,
# COMMENT_USER, COMMENT_USER_ID, COMMENT_ID.
#
# `cla-check.sh --selftest` checks the decision logic without the network.

set -euo pipefail

SIGN_TEXT='I have read the VeridianOS CLA Document and I hereby sign the CLA'
BRANCH='cla-signatures'
FILE='signatures/version1/cla.json'
DOC_URL='https://github.com/doublegate/VeridianOS/blob/main/CLA.md'
MARKER='<!-- veridianos-cla-check -->'
# The maintainer holds the rights already; bots author nothing copyrightable.
ALLOW_RE='^(doublegate|.+\[bot\])$'

# The authors of a pull request who must sign: `committers` minus the
# allowlist minus `signed` (each one login per line). An author without a
# linked GitHub account is listed as "@unlinked:<name>" and can never match.
unsigned_of() {
    local committers="$1" signed="$2"
    comm -23 \
        <(printf '%s\n' "$committers" | sed '/^$/d' | grep -Ev "$ALLOW_RE" | LC_ALL=C sort -u) \
        <(printf '%s\n' "$signed" | sed -e '/^$/d' -e '/^@unlinked:/d' | LC_ALL=C sort -u) || true
}

# A comment body without carriage returns, surrounding whitespace, or the
# "> " of a quotation copied from the check's own comment.
normalize_comment() {
    printf '%s' "$1" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
        | sed -e '1s/^>[[:space:]]*//' | sed -e '/^[[:space:]]*$/d'
}

# Whether a comment body is the signing text.
is_sign_comment() {
    [ "$(normalize_comment "$1")" = "$SIGN_TEXT" ]
}

# Whether a comment body asks for a new check.
is_recheck_comment() {
    [ "$(normalize_comment "$1")" = "recheck" ]
}

# The signed logins in a signatures document.
signed_of() {
    printf '%s' "$1" | jq -r '.signedContributors[]?.name // empty'
}

# A signatures document with one more signature.
add_signature() {
    local doc="$1" login="$2" id="$3" comment_id="$4" pr="$5" repo_id="$6" now="$7"
    printf '%s' "$doc" | jq \
        --arg name "$login" --argjson id "$id" --argjson cid "$comment_id" \
        --argjson pr "$pr" --argjson rid "$repo_id" --arg at "$now" \
        '.signedContributors += [{name: $name, id: $id, comment_id: $cid,
          created_at: $at, repoId: $rid, pullRequestNo: $pr}]'
}

selftest() {
    local fails=0
    check() { if [ "$2" = "$3" ]; then echo "  ok    $1"; else echo "  FAIL  $1: got [$2] want [$3]"; fails=$((fails + 1)); fi; }
    check "an unsigned author is listed" "$(unsigned_of $'alice\nbob' 'alice')" "bob"
    check "the maintainer and bots need not sign" \
        "$(unsigned_of $'doublegate\ndependabot[bot]\ngithub-actions[bot]' '')" ""
    check "authors are listed once" "$(unsigned_of $'bob\nbob\nbob' '')" "bob"
    check "an unlinked author can never match" \
        "$(unsigned_of '@unlinked:Bob Smith' '@unlinked:Bob Smith')" "@unlinked:Bob Smith"
    check "a login that only contains a bot name is not a bot" \
        "$(unsigned_of 'evil[bot]x' '')" "evil[bot]x"
    check "everyone signed" "$(unsigned_of $'alice\nbob' $'bob\nalice')" ""
    is_sign_comment "$SIGN_TEXT" && r=yes || r=no
    check "the signing text signs" "$r" "yes"
    is_sign_comment $'  '"$SIGN_TEXT"$'\r\n' && r=yes || r=no
    check "surrounding whitespace is ignored" "$r" "yes"
    is_sign_comment "$SIGN_TEXT and more" && r=yes || r=no
    check "anything more does not sign" "$r" "no"
    is_sign_comment "recheck" && r=yes || r=no
    check "recheck does not sign" "$r" "no"
    is_sign_comment "> $SIGN_TEXT"$'\n' && r=yes || r=no
    check "a pasted quotation signs" "$r" "yes"
    is_sign_comment "I do not sign. $SIGN_TEXT" && r=yes || r=no
    check "text before the signing text does not sign" "$r" "no"
    is_recheck_comment $'recheck\r\n' && r=yes || r=no
    check "recheck with a newline rechecks" "$r" "yes"
    is_recheck_comment "please do not recheck" && r=yes || r=no
    check "a sentence containing recheck does not" "$r" "no"
    local doc='{"signedContributors":[]}'
    doc="$(add_signature "$doc" 'alice' 7 99 48 123 '2026-11-15T00:00:00Z')"
    check "a signature is recorded" "$(signed_of "$doc")" "alice"
    check "its fields" "$(printf '%s' "$doc" | jq -c '.signedContributors[0] | [.id,.comment_id,.pullRequestNo,.repoId]')" "[7,99,48,123]"
    doc="$(add_signature "$doc" 'x"; rm -rf /' 8 100 48 123 'now')"
    check "a hostile login stays data" "$(signed_of "$doc" | tail -1)" 'x"; rm -rf /'
    if [ "$fails" -eq 0 ]; then echo "all checks passed"; else echo "$fails check(s) failed"; return 1; fi
}

if [ "${1:-}" = "--selftest" ]; then
    selftest
    exit $?
fi

: "${GH_TOKEN:?}" "${REPO:?}" "${PR:?}" "${EVENT:?}"
# The workflow's filter is loose; a comment that is neither text does nothing.
if [ "$EVENT" = "issue_comment" ] && ! is_sign_comment "${COMMENT_BODY:-}" \
    && ! is_recheck_comment "${COMMENT_BODY:-}"; then
    echo "CLA: comment ignored (neither the signing text nor recheck)"
    exit 0
fi
case "$PR" in '' | *[!0-9]*) echo "PR is not a number" >&2; exit 1 ;; esac
owner="${REPO%%/*}"
name="${REPO#*/}"

# Every commit author, all pages (GraphQL, 100 commits a page).
# shellcheck disable=SC2016 # GraphQL variables, not shell ones
committers="$(gh api graphql --paginate \
    -F owner="$owner" -F name="$name" -F number="$PR" \
    -f query='query($owner: String!, $name: String!, $number: Int!, $endCursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          commits(first: 100, after: $endCursor) {
            nodes { commit { author { name user { login } } } }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }' \
    --jq '.data.repository.pullRequest.commits.nodes[].commit.author
          | .user.login // (if ((.name // "") | endswith("[bot]")) then .name
                            else "@unlinked:" + (.name // "unknown") end)')"

# The signatures, or none yet. Only "not found" counts as empty: any other
# failure stops the check (fail closed).
read_signatures() {
    local out
    if out="$(gh api "repos/$REPO/contents/$FILE?ref=$BRANCH" 2>&1)"; then
        sig_sha="$(printf '%s' "$out" | jq -r .sha)"
        sig_doc="$(printf '%s' "$out" | jq -r .content | base64 -d)"
    elif printf '%s' "$out" | grep -q 'Not Found\|HTTP 404'; then
        sig_sha=''
        sig_doc='{"signedContributors":[]}'
    else
        echo "cannot read the signatures: $out" >&2
        exit 1
    fi
}
read_signatures

# A signature in this comment: recorded once, with a retry if another run
# changed the file in between (the workflow also serializes runs).
if [ "$EVENT" = "issue_comment" ] && is_sign_comment "${COMMENT_BODY:-}"; then
    : "${COMMENT_USER:?}" "${COMMENT_USER_ID:?}" "${COMMENT_ID:?}"
    for attempt in 1 2 3; do
        if signed_of "$sig_doc" | grep -Fxq -- "$COMMENT_USER"; then
            break
        fi
        repo_id="$(gh api "repos/$REPO" --jq .id)"
        new_doc="$(add_signature "$sig_doc" "$COMMENT_USER" "$COMMENT_USER_ID" "$COMMENT_ID" \
            "$PR" "$repo_id" "$(date -u +%Y-%m-%dT%H:%M:%SZ)")"
        args=(-X PUT "repos/$REPO/contents/$FILE"
            -f message="CLA: @$COMMENT_USER signs (pull request #$PR)"
            -f content="$(printf '%s\n' "$new_doc" | base64 -w0)"
            -f branch="$BRANCH")
        [ -n "$sig_sha" ] && args+=(-f sha="$sig_sha")
        if gh api "${args[@]}" >/dev/null; then
            sig_doc="$new_doc"
            break
        fi
        [ "$attempt" = 3 ] && { echo "could not record the signature" >&2; exit 1; }
        sleep 2
        read_signatures
    done
fi

unsigned="$(unsigned_of "$committers" "$(signed_of "$sig_doc")")"
head_sha="$(gh api "repos/$REPO/pulls/$PR" --jq .head.sha)"

if [ -z "$unsigned" ]; then
    state=success
    desc='Every author has signed the CLA'
    body="$MARKER"$'\n'"All authors of this pull request have signed the [VeridianOS Contributor Licence Agreement]($DOC_URL). Thank you."
else
    state=failure
    desc='Waiting for CLA signatures'
    # shellcheck disable=SC2016 # literal Markdown backticks
    list="$(printf '%s\n' "$unsigned" | sed -e 's/^@unlinked:\(.*\)/- \1 (a commit author with no GitHub account linked: add the commit e-mail address to a GitHub account, then comment `recheck`)/' -e '/^- /!s/^/- @/')"
    body="$MARKER"$'\n'"Thank you for your contribution. Before it can be merged, each author below needs to read the [VeridianOS Contributor Licence Agreement]($DOC_URL) and sign it by posting this comment:"$'\n\n'"> $SIGN_TEXT"$'\n\n'"You sign once; it covers your later contributions. The CLA keeps your copyright with you and lets the project offer its code under both the GPL and commercial licences (ADR 0028)."$'\n\n'"Not signed yet:"$'\n'"$list"
fi

gh api -X POST "repos/$REPO/statuses/$head_sha" \
    -f state="$state" -f context='CLA' -f description="$desc" -f target_url="$DOC_URL" >/dev/null

# One comment, kept up to date.
existing="$(gh api --paginate "repos/$REPO/issues/$PR/comments" \
    --jq '.[] | select(.user.login == "github-actions[bot]" and (.body | contains("<!-- veridianos-cla-check -->"))) | .id' | head -1)"
if [ -n "$existing" ]; then
    gh api -X PATCH "repos/$REPO/issues/comments/$existing" -f body="$body" >/dev/null
elif [ -n "$unsigned" ]; then
    gh api -X POST "repos/$REPO/issues/$PR/comments" -f body="$body" >/dev/null
fi

echo "CLA: $state${unsigned:+ (unsigned: $(printf '%s' "$unsigned" | tr '\n' ' '))}"
[ "$state" = success ]
