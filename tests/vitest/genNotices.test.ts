import { describe, expect, it } from 'vitest'

// @ts-expect-error 打包脚本是无类型的 .mjs，这里只测它导出的纯函数
import { collectNpm } from '../../scripts/gen-notices.mjs'

type Node = { version: string; path?: string; dependencies?: Record<string, Node>; optionalDependencies?: Record<string, Node> }

const names = (roots: unknown[]) =>
  (collectNpm(roots) as { name: string; version: string }[]).map((p) => `${p.name}@${p.version}`).sort()

describe('collectNpm', () => {
  it('收集嵌套的传递依赖', () => {
    const roots = [
      {
        dependencies: {
          a: { version: '1.0.0', dependencies: { b: { version: '2.0.0' } } },
        } as Record<string, Node>,
      },
    ]
    expect(names(roots)).toEqual(['a@1.0.0', 'b@2.0.0'])
  })

  /**
   * pnpm 的 list JSON 会对重复出现的子树去重：同一个包出现多次时，可能一次只有
   * 一个壳、另一次才带完整的 dependencies。哪一次在前取决于树的形状。
   *
   * 曾经的实现把递归写在「首次见到」的分支里，于是先撞上空壳的那次就把包记下了，
   * 后面带完整子树的那次被跳过，整棵子树永久丢失。
   *
   * 这个形态不是构造出来的：同一份代码在本地收到 257 个组件、在 CI 上只有 254，
   * 少的正是 react / react-dom 底下的 loose-envify、js-tokens、scheduler。
   */
  it('同一个包先以空壳出现、后带完整子树时，子树不能丢', () => {
    const roots = [
      {
        dependencies: {
          // 先出现的是空壳
          react: { version: '18.3.1' },
          // 后出现的这棵才带着传递依赖
          wrapper: {
            version: '1.0.0',
            dependencies: {
              react: {
                version: '18.3.1',
                dependencies: {
                  'loose-envify': {
                    version: '1.4.0',
                    dependencies: { 'js-tokens': { version: '4.0.0' } },
                  },
                },
              },
            },
          },
        } as Record<string, Node>,
      },
    ]

    expect(names(roots)).toEqual([
      'js-tokens@4.0.0',
      'loose-envify@1.4.0',
      'react@18.3.1',
      'wrapper@1.0.0',
    ])
  })

  it('optionalDependencies 也要跟进去', () => {
    const roots = [
      {
        dependencies: {
          a: { version: '1.0.0', optionalDependencies: { opt: { version: '3.0.0' } } },
        } as Record<string, Node>,
      },
    ]
    expect(names(roots)).toEqual(['a@1.0.0', 'opt@3.0.0'])
  })

  it('同名同版本只收一次', () => {
    const roots = [
      {
        dependencies: {
          a: { version: '1.0.0', dependencies: { dup: { version: '9.9.9' } } },
          b: { version: '1.0.0', dependencies: { dup: { version: '9.9.9' } } },
        } as Record<string, Node>,
      },
    ]
    expect(names(roots)).toEqual(['a@1.0.0', 'b@1.0.0', 'dup@9.9.9'])
  })

  it('空输入不炸', () => {
    expect(names([])).toEqual([])
    expect(names([{}])).toEqual([])
    expect(names([{ dependencies: undefined }])).toEqual([])
  })
})
