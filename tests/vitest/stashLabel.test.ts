import { describe, expect, it } from 'vitest'
import { shortenStashMessage } from '../../src/engine/stashLabel'

describe('shortenStashMessage', () => {
  it('剥掉自动生成的 WIP 前缀和提交 hash', () => {
    expect(shortenStashMessage('WIP on feature: da964eb f2-rewritten')).toBe('f2-rewritten')
  })

  it('剥掉带 -m 时的 On 前缀，保留用户写的说明', () => {
    expect(shortenStashMessage('On master: 改到一半的登录页')).toBe('改到一半的登录页')
  })

  it('分支名里有斜杠也能剥干净', () => {
    expect(shortenStashMessage('WIP on feat/git-ops: abc1234 wip')).toBe('wip')
  })

  it('保留说明里的冒号', () => {
    expect(shortenStashMessage('On master: fix: 空指针')).toBe('fix: 空指针')
  })

  it('认不出格式就原样返回，不要丢信息', () => {
    expect(shortenStashMessage('随便写的一句话')).toBe('随便写的一句话')
  })

  it('剥完是空的就退回原文，不要给出一个空标题', () => {
    // 空行在侧边栏里就是一条点不明白的空白行
    expect(shortenStashMessage('WIP on master: ')).toBe('WIP on master: ')
  })

  it('只剩一个 hash 时就显示 hash——那仍然是有效信息', () => {
    expect(shortenStashMessage('WIP on master: da964eb')).toBe('da964eb')
  })
})
