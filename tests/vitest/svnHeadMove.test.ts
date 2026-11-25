import { describe, expect, it } from 'vitest'
import {
  canApplyFollowedSync,
  isPlainCommitOn,
  sameHeadForSync,
  type HeadObservation,
} from '@/engine/svnSync'

// 第二档里只有 `behind_fork` 看 HEAD：处境（游离 / 基线分支 / 其余分支）+ merge-base(HEAD, svn/base)。
// 2026-11-11 提交慢排查 H1。

const O = 'o'.repeat(40)
const P = 'p'.repeat(40)
const N = 'n'.repeat(40)

const atP: HeadObservation = { branch: 'work', hash: P, parents: [O] }

describe('sameHeadForSync', () => {
  it('同一个提交、同一个分支：相同', () => {
    expect(sameHeadForSync(atP, { ...atP })).toBe(true)
  })

  it('同一个提交换了一条普通分支（checkout -b）：结论不变，算相同', () => {
    expect(sameHeadForSync(atP, { ...atP, branch: 'feature' })).toBe(true)
  })

  it('同一个提交切到基线分支或游离：处境变了，不同', () => {
    expect(sameHeadForSync(atP, { ...atP, branch: 'svn/base' })).toBe(false)
    expect(sameHeadForSync(atP, { ...atP, branch: 'HEAD' })).toBe(false)
  })

  it('提交不同：不同', () => {
    expect(sameHeadForSync(atP, { branch: 'work', hash: N, parents: [P] })).toBe(false)
  })

  it('只是名字像基线的分支不算基线', () => {
    expect(sameHeadForSync(atP, { ...atP, branch: 'svn/base-backup' })).toBe(true)
  })
})

describe('isPlainCommitOn', () => {
  it('同一条分支上、唯一的父提交就是原来的 HEAD：是', () => {
    expect(isPlainCommitOn(atP, { branch: 'work', hash: N, parents: [P] })).toBe(true)
  })

  it('游离 HEAD 上的提交也算：behind_fork 两边都按不落后', () => {
    const detached = { ...atP, branch: 'HEAD' }
    expect(isPlainCommitOn(detached, { branch: 'HEAD', hash: N, parents: [P] })).toBe(true)
  })

  it('amend：父提交不是原来的 HEAD，不是', () => {
    expect(isPlainCommitOn(atP, { branch: 'work', hash: N, parents: [O] })).toBe(false)
  })

  it('解决 merge 后的提交：两个父提交（第二个可能就是基线），不是', () => {
    expect(isPlainCommitOn(atP, { branch: 'work', hash: N, parents: [P, O] })).toBe(false)
  })

  it('分支变了：不是', () => {
    expect(isPlainCommitOn(atP, { branch: 'other', hash: N, parents: [P] })).toBe(false)
  })

  it('在基线分支本身上提交（基线 OID 动了）：不是', () => {
    const onBase = { ...atP, branch: 'svn/base' }
    expect(isPlainCommitOn(onBase, { branch: 'svn/base', hash: N, parents: [P] })).toBe(false)
  })

  it('根提交（没有父提交）：不是', () => {
    expect(isPlainCommitOn(atP, { branch: 'work', hash: N, parents: [] })).toBe(false)
  })
})

describe('canApplyFollowedSync（跟进带回判定，2026-11-24 分析方案 4）', () => {
  const atN: HeadObservation = { branch: 'work', hash: N, parents: [P] }
  const quiet = { writtenSinceFollow: false, stampsKnown: true, pending: null }

  it('跟进期间没人写回、戳记过、没有排着的校验：写回，记带回的 HEAD', () => {
    expect(canApplyFollowedSync(quiet, atN)).toBe(true)
  })

  it('排着的就是同一个 HEAD：写回（那一轮被顶掉也无妨，结论相同）', () => {
    expect(canApplyFollowedSync({ ...quiet, pending: { ...atN, branch: 'feature' } }, atN)).toBe(true)
  })

  it('排着的是别的 HEAD：不写，写了会把那一轮顶掉、没人再替它校验', () => {
    expect(canApplyFollowedSync({ ...quiet, pending: atP }, atN)).toBe(false)
    expect(canApplyFollowedSync({ ...quiet, pending: { ...atN, branch: 'HEAD' } }, atN)).toBe(false)
  })

  it('跟进期间已有检测写回：不写，那一轮可能晚于锁内复核、还记下了那一刻的戳', () => {
    expect(canApplyFollowedSync({ ...quiet, writtenSinceFollow: true }, atN)).toBe(false)
  })

  it('戳还没记过：不写，否则之后没人补记、失焦快照不比戳', () => {
    expect(canApplyFollowedSync({ ...quiet, stampsKnown: false }, atN)).toBe(false)
  })
})
