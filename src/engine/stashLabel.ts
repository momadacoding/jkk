/**
 * 把 git 的 stash 描述压成一行能看的短标题。
 *
 * git 生成的是 `WIP on feature: da964eb f2-rewritten`——「WIP on <分支>: <提交>」
 * 这段前缀每条都长得差不多，在只有一两百像素宽的侧边栏里会把真正有用的那部分
 * 挤没。带 -m 存的则是 `On feature: 我的说明`。
 *
 * 认不出格式就原样返回，宁可长一点也不要丢信息。
 */
export function shortenStashMessage(message: string): string {
  const match = /^(?:WIP on|On)\s+[^:]+:\s*(?:[0-9a-f]{7,40}\s+)?(.*)$/.exec(message)
  const rest = match?.[1]?.trim()
  return rest ? rest : message
}
