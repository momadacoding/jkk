import { describe, expect, it } from 'vitest'
import {
  driverBanners,
  mergeRiskConfirm,
  triggersMerge,
  type DriverFinding,
  type DriverReport,
} from '@/engine/mergeDrivers'

function report(findings: DriverFinding[], incomplete: string | null = null): DriverReport {
  return { findings, incomplete }
}

const missing: DriverFinding = {
  name: 'unityyamlmerge',
  state: { kind: 'missing' },
  samplePaths: ['Main.unity', 'Other.unity'],
}
const incomplete: DriverFinding = {
  name: 'unityyamlmerge',
  state: { kind: 'incomplete' },
  samplePaths: ['Main.unity'],
}
const programMissing: DriverFinding = {
  name: 'unityyamlmerge',
  state: { kind: 'programMissing', program: 'C:/nope/NotThere.exe' },
  samplePaths: ['Main.unity'],
}
const ok: DriverFinding = { name: 'good', state: { kind: 'ok' }, samplePaths: ['a.unity'] }

describe('driverBanners', () => {
  it('完全没配：红色横幅，附上配置示例', () => {
    const [b] = driverBanners(report([missing]))
    expect(b.tone).toBe('danger')
    expect(b.text).toBe(
      '.gitattributes 让 Main.unity 等文件用 merge driver unityyamlmerge，但本机 git 配置里没有定义它。'
        + '在这台机器上合并时，git 会往这些文件里写冲突标记，Unity 场景会被写坏。',
    )
    expect(b.example).toContain('[merge "unityyamlmerge"]')
    expect(b.example).toContain('--fallback none %O %B %A %A')
  })

  it('没有示例路径（只来自 merge.default）时不编造路径', () => {
    const [b] = driverBanners(report([{ ...missing, samplePaths: [] }]))
    expect(b.tone).toBe('danger')
    expect(b.text).toContain('merge driver unityyamlmerge')
    expect(b.text).not.toContain('undefined')
  })

  it('配置不完整：红色横幅', () => {
    expect(driverBanners(report([incomplete]))).toEqual([{
      tone: 'danger',
      text: 'merge driver unityyamlmerge 只配了一部分，缺少 driver 命令。在这台机器上，所有涉及这类文件的合并都会直接失败。',
    }])
  })

  it('程序不存在：琥珀色横幅', () => {
    expect(driverBanners(report([programMissing]))).toEqual([{
      tone: 'warning',
      text: 'merge driver unityyamlmerge 指向的程序不存在：C:/nope/NotThere.exe。合并时它会失败，冲突需要手动解决。',
    }])
  })

  it('检查未完成：说明原因，不能当成配置正常', () => {
    expect(driverBanners(report([], 'git 不在 PATH 上'))).toEqual([{
      tone: 'info',
      text: 'merge driver 检查未完成：git 不在 PATH 上',
    }])
  })

  it('全部 Ok、还没读到时都不挂横幅', () => {
    expect(driverBanners(report([ok]))).toEqual([])
    expect(driverBanners(null)).toEqual([])
  })

  it('多个问题各挂一条，Ok 的跳过', () => {
    const tones = driverBanners(report([ok, missing, { ...programMissing, name: 'other' }]))
      .map((b) => b.tone)
    expect(tones).toEqual(['danger', 'warning'])
  })
})

describe('mergeRiskConfirm', () => {
  it('只在完全没配时拦：危险样式，确认按钮是「仍然继续」，默认焦点在取消', () => {
    const req = mergeRiskConfirm(report([missing]))
    expect(req).not.toBeNull()
    expect(req!.danger).toBe(true)
    expect(req!.confirmLabel).toBe('仍然继续')
    expect(req!.focusCancel).toBe(true)
    expect(req!.message).toContain('unityyamlmerge')
    expect(req!.warning).toContain('冲突标记')
  })

  it('配置不完整、程序不存在、检查未完成、全部正常都不拦', () => {
    expect(mergeRiskConfirm(report([incomplete]))).toBeNull()
    expect(mergeRiskConfirm(report([programMissing]))).toBeNull()
    expect(mergeRiskConfirm(report([], '失败'))).toBeNull()
    expect(mergeRiskConfirm(report([ok]))).toBeNull()
  })

  it('多个缺失的名字都写进说明', () => {
    const req = mergeRiskConfirm(report([missing, { ...missing, name: 'prefabmerge' }]))
    expect(req!.message).toContain('unityyamlmerge')
    expect(req!.message).toContain('prefabmerge')
  })
})

describe('triggersMerge', () => {
  it('会让 git 跑合并的命令', () => {
    for (const cmd of ['pull', 'cherry_pick', 'revert', 'stash_apply', 'stash_pop']) {
      expect(triggersMerge(cmd)).toBe(true)
    }
  })

  it('不合并的命令不拦', () => {
    for (const cmd of ['fetch', 'push', 'push_tag', 'stash_push', 'stash_drop', 'checkout', 'stage', 'commit']) {
      expect(triggersMerge(cmd)).toBe(false)
    }
  })
})
