import type { ReactNode } from 'react'
import type { GraphLayout, RefPointer } from './graphTypes'

export interface GraphRenderer {
  render(layout: GraphLayout, refs: RefPointer[], selectedId: string | null): ReactNode
}
