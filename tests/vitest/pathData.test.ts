import { describe, expect, it } from 'vitest'
import { toPathData } from '../../src/engine/pathData'
import type { PathSegment } from '../../src/engine/graphTypes'

const ROW = 24

/** 从 path 数据里抠出所有坐标点，方便断言形状。 */
function points(d: string): Array<[number, number]> {
  return [...d.matchAll(/(-?[\d.]+)\s+(-?[\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])])
}

describe('toPathData', () => {
  it('同车道内是一条直线', () => {
    const seg: PathSegment = { x1: 10, y1: 12, x2: 10, y2: 60, curve: false, cornerAtTop: false, dashed: false }
    expect(toPathData([seg], ROW).trim()).toBe('M 10 12 L 10 60')
  })

  it('跨车道时不出现圆弧指令', () => {
    // 圆角用 Q，斜线只该有 M/L
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 108, curve: true, cornerAtTop: false, dashed: false }
    const d = toPathData([seg], ROW)
    expect(d).not.toMatch(/[QqCcAa]/)
    expect(d).toMatch(/^[ML\s\d.-]+$/)
  })

  it('第一父：先在本车道竖下来，最后一行斜过去', () => {
    // 子在 lane0 第 0 行，父在 lane1 第 4 行
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 108, curve: true, cornerAtTop: false, dashed: false }
    const pts = points(toPathData([seg], ROW))
    expect(pts[0]).toEqual([10, 12])
    // 竖到倒数第二行才开始斜
    expect(pts[1]).toEqual([10, 108 - ROW])
    expect(pts[2]).toEqual([34, 108])
  })

  it('第二父：从子节点立刻斜进父车道，再一路竖下去', () => {
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 108, curve: true, cornerAtTop: true, dashed: false }
    const pts = points(toPathData([seg], ROW))
    expect(pts[0]).toEqual([10, 12])
    expect(pts[1]).toEqual([34, 12 + ROW])
    expect(pts[2]).toEqual([34, 108])
  })

  it('往左跨车道同样是斜线', () => {
    const seg: PathSegment = { x1: 34, y1: 12, x2: 10, y2: 108, curve: true, cornerAtTop: false, dashed: false }
    const pts = points(toPathData([seg], ROW))
    expect(pts[1]).toEqual([34, 108 - ROW])
    expect(pts[2]).toEqual([10, 108])
  })

  it('父子相邻时整段就是一条斜线，不留多余顶点', () => {
    // 斜线占满一行高，中间那个转折点正好落在起点上，画出来会是一个零长竖线
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 36, curve: true, cornerAtTop: false, dashed: false }
    const pts = points(toPathData([seg], ROW))
    expect(pts).toEqual([[10, 12], [34, 36]])
  })

  it('相邻的第二父同样只有两个点', () => {
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 36, curve: true, cornerAtTop: true, dashed: false }
    expect(points(toPathData([seg], ROW))).toEqual([[10, 12], [34, 36]])
  })

  it('斜线不会超过整段高度', () => {
    // 万一父子距离小于一行高（不该发生，但坐标是外部算的），
    // 斜线要收敛到实际距离，不能画出反向的折返
    const seg: PathSegment = { x1: 10, y1: 12, x2: 34, y2: 24, curve: true, cornerAtTop: false, dashed: false }
    const pts = points(toPathData([seg], ROW))
    expect(pts.every(([, y]) => y >= 12 && y <= 24)).toBe(true)
  })

  it('多段拼在一起', () => {
    const segs: PathSegment[] = [
      { x1: 10, y1: 12, x2: 10, y2: 36, curve: false, cornerAtTop: false, dashed: false },
      { x1: 10, y1: 36, x2: 34, y2: 84, curve: true, cornerAtTop: false, dashed: false },
    ]
    const d = toPathData(segs, ROW)
    expect(d.match(/M /g)).toHaveLength(2)
  })
})
