import { describe, expect, it } from 'vitest'
import { createAttentionTracker, describeAttention } from '@/engine/attention'

const blocked = (...entries: [string, string][]) =>
  new Map(entries.map(([path, reason]) => [path, { path, reason }]))

describe('describeAttention（使用报告 9.5）', () => {
  it('没有被挡下的仓库：没有提醒', () => {
    expect(describeAttention(blocked())).toBeNull()
  })

  it('一个仓库：仓库名 + 原因，仓库名保留大小写', () => {
    expect(describeAttention(blocked(['D:\\Program\\trunk\\Client', '重放有冲突（A.lua）']))).toBe(
      'Client 没有自动跟进：重放有冲突（A.lua）',
    )
  })

  it('几个仓库：写个数和名字，原因太长放不下，点开窗口看', () => {
    expect(describeAttention(blocked(['C:/a/Client', 'x'], ['C:/b/Server', 'y']))).toBe(
      '2 个仓库没有自动跟进：Client、Server',
    )
  })

  it('同名的仓库（同一个工程的几份工作副本）多带一级目录，分得出是哪个（独立 review 第 8 条）', () => {
    expect(
      describeAttention(blocked(['D:/trunk/Client', 'x'], ['D:\\branches\\x\\Client', 'y'])),
    ).toBe('2 个仓库没有自动跟进：trunk/Client、x/Client')
  })

  it('只有一个被挡下，但开着同名的别的仓库：名字照样只写一级（同名只在被挡下的几个之间比）', () => {
    expect(describeAttention(blocked(['D:/trunk/Client', '冲突']))).toBe('Client 没有自动跟进：冲突')
  })
})

/** 记下每次发给后端的 (原因, 要不要闪)。 */
function tracked() {
  const sent: [string | null, boolean][] = []
  const tracker = createAttentionTracker((reason, flash) => sent.push([reason, flash]))
  return { sent, tracker }
}

describe('createAttentionTracker', () => {
  it('第一次被挡下：挂提醒并闪一次', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/wc/Client', '重放有冲突')
    expect(sent).toEqual([['Client 没有自动跟进：重放有冲突', true]])
  })

  it('同一个原因重复上报（组件重新渲染）：不再发', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/wc/Client', '重放有冲突')
    tracker.report('C:/wc/Client', '重放有冲突')
    expect(sent).toHaveLength(1)
  })

  it('原因换了：再闪一次，那是一件新的事', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/wc/Client', '重放有冲突')
    tracker.report('C:/wc/Client', '没能向服务器核对新基线')
    expect(sent.at(-1)).toEqual(['Client 没有自动跟进：没能向服务器核对新基线', true])
  })

  it('原因没了（跟进了、条件变了）：撤掉，不闪', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/wc/Client', '重放有冲突')
    tracker.report('C:/wc/Client', null)
    expect(sent.at(-1)).toEqual([null, false])
  })

  it('从来没挂过：上报「没有」不发任何东西', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/wc/Client', null)
    tracker.report('C:/wc/Client', '')
    tracker.report('C:/wc/Client', '   ')
    expect(sent).toEqual([])
  })

  it('路径写法不同（大小写、反斜杠、尾部斜杠）算同一个仓库', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:\\WC\\Client', '重放有冲突')
    tracker.report('c:/wc/client/', null)
    expect(sent.at(-1)).toEqual([null, false])
  })

  it('几个仓库时其中一个换了原因：汇总文字不变，也要再闪一次（独立 review 第 3 条）', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/a/Client', '重放有冲突')
    tracker.report('C:/b/Server', 'y')
    const before = sent.length
    tracker.report('C:/a/Client', '没能向服务器核对新基线')
    expect(sent.slice(before)).toEqual([['2 个仓库没有自动跟进：Client、Server', true]])
  })

  it('两个标签各有原因：一个撤了，另一个的提醒还在，不为剩下的那个再闪', () => {
    const { sent, tracker } = tracked()
    tracker.report('C:/a/Client', 'x')
    tracker.report('C:/b/Server', 'y')
    expect(sent.at(-1)).toEqual(['2 个仓库没有自动跟进：Client、Server', true])
    tracker.report('C:/a/Client', null)
    expect(sent.at(-1)).toEqual(['Server 没有自动跟进：y', false])
    tracker.report('C:/b/Server', null)
    expect(sent.at(-1)).toEqual([null, false])
  })
})
