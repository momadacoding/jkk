#!/usr/bin/env bash
# 生成 Release 说明里的「新功能 / 修复」两节，输出到 stdout（日志走 stderr）。
#
#   bash scripts/release-notes.sh v0.6.0           # 已经打好的 tag
#   bash scripts/release-notes.sh v0.7.0 master    # 打 tag 之前，拿 origin 上的 master 预览
#
# release workflow 用它生成说明；本地预览走的是同一份逻辑。
#
# 内容取自「上一个正式版 tag...本 tag」之间的提交标题：feat 进「新功能」，fix 进「修复」，
# 其余（docs / chore / refactor ...）不列。走 GitHub API 而不是 git log：
# CI 的 checkout 是浅克隆、又关掉了凭据，取不到更早的 tag 和历史。
set -Eeuo pipefail
trap 'echo "[release-notes] 第 $LINENO 行出错" >&2' ERR

log() { echo "[release-notes] $*" >&2; }

tag="${1:?用法：release-notes.sh <tag> [比对终点，默认就是 tag]}"
head="${2:-$tag}"
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "tag 名不是合法的 SemVer：$tag" >&2
  exit 1
fi

# 上一个正式版：只在不带预发布后缀的 tag 里找，取版本序排在本 tag 前面的那个。
# 把本 tag 也放进去一起排序，再取它的前一行；本 tag 排第一说明没有更早的正式版。
# awk 不提前 exit：管道里任何一段提前退出都会让上游吃 SIGPIPE，在 pipefail 下变成失败。
log "列出仓库 tag"
all_tags=$(gh api --paginate 'repos/{owner}/{repo}/tags' --jq '.[].name')
prev=$(
  { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' <<<"$all_tags" || true; echo "$tag"; } |
    sort -uV | awk -v t="$tag" '$0 == t { print p } { p = $0 }'
)
if [[ -z "$prev" ]]; then
  log "$tag 之前没有正式版 tag，不生成改动列表"
  exit 0
fi
log "比对 $prev...$head"

# 一次请求取齐：第 1 行网页链接，第 2 行总提交数，第 3 行本页提交数，之后每行一个提交标题
out=$(gh api "repos/{owner}/{repo}/compare/$prev...$head" \
  --jq '.html_url, .total_commits, (.commits | length), (.commits[].commit.message | split("\n")[0])')
url=$(sed -n 1p <<<"$out")
total=$(sed -n 2p <<<"$out")
got=$(sed -n 3p <<<"$out")
subjects=$(sed -n '4,$p' <<<"$out")

# 只认 Conventional Commits 的 type 前缀。`feat(svn): xxx (#59)` 写成 `- svn：xxx (#59)`：
# scope 要留着，「Phase 3a 交付」这种标题离开 svn 两个字就看不出是什么
section() {
  local type=$1
  { grep -E "^$type(\([^)]*\))?!?: " <<<"$subjects" || true; } |
    sed -E -e "s/^$type\(([^)]*)\)!?: /- \1：/" -e "s/^$type!?: /- /"
}
feats=$(section feat)
fixes=$(section fix)

if [[ -n "$feats" ]]; then
  printf '## 新功能\n%s\n\n' "$feats"
fi
if [[ -n "$fixes" ]]; then
  printf '## 修复\n%s\n\n' "$fixes"
fi
if [[ -z "$feats$fixes" ]]; then
  printf '本版本没有新功能或修复类改动。\n\n'
fi
# compare 接口单页最多返回 250 个提交，超出时如实说明，不假装列全了
if (( got < total )); then
  printf '（共 %s 个提交，以上只按前 %s 个整理。）\n\n' "$total" "$got"
fi
printf '完整提交记录：%s\n' "$url"
