import type { PathSegment } from './graphTypes'

/**
 * 把线段转成 SVG path 数据。
 *
 * 跨车道用**斜线**过渡，和 Sublime Merge 一致：线在自己的车道里笔直地走，
 * 只在靠近另一端时用一段占一行高的斜线切过去。
 *
 * 这里换过两版。最早是跨越整段高度的贝塞尔——相邻两行还好看，一旦跨十几行就
 * 变成一道横扫几百像素的巨大弧线，完全看不出它连的是哪两个提交。第二版改成
 * 直线加小圆角，形状对了但拐角处像个方框。斜线两者都避开：既紧凑，方向也一眼可读。
 *
 * @param rowHeight 一行的高度，斜线正好占一行——这是让所有分叉/合并的斜率一致的关键
 */
export function toPathData(segments: PathSegment[], rowHeight: number): string {
  let d = ''
  for (const s of segments) {
    if (!s.curve) {
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${s.y2} `
      continue
    }
    // 父子相邻时整段距离就等于一行，斜线占满全程，中间不该再插一个转折点——
    // 那会画出一条零长度的竖线
    const span = Math.abs(s.y2 - s.y1)
    const diag = Math.min(rowHeight, span)

    if (s.cornerAtTop) {
      // 第二父：从子节点立刻斜进父提交的车道，再一路竖下去
      const turn = s.y1 + diag
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${turn} `
      if (turn !== s.y2) d += `L ${s.x2} ${s.y2} `
    } else {
      // 第一父：在子节点的车道里竖下来，最后一行才斜过去
      const turn = s.y2 - diag
      d += `M ${s.x1} ${s.y1} `
      if (turn !== s.y1) d += `L ${s.x1} ${turn} `
      d += `L ${s.x2} ${s.y2} `
    }
  }
  return d
}
