/** 一个 merge driver 在本机的配置状态，与 Rust 侧 `DriverState` 一一对应（`kind` 做标签）。 */
export type DriverState =
  | { kind: 'ok' }
  | { kind: 'missing' }
  | { kind: 'incomplete' }
  | { kind: 'programMissing'; program: string }

export interface DriverFinding {
  name: string
  state: DriverState
  /** 用这个 driver 的路径，最多 3 条。只来自 `merge.default` 时可能为空。 */
  samplePaths: string[]
}

/** `check_merge_drivers` 的结果。`incomplete` 非 null 表示检查本身失败。 */
export interface DriverReport {
  findings: DriverFinding[]
  incomplete: string | null
}

export interface DriverBanner {
  tone: 'danger' | 'warning' | 'info'
  text: string
  /** 可以复制的配置示例（只有「完全没配」才有）。 */
  example?: string
}

/** spec §4.6 给用户看的配置示例，名字换成实际缺的那个。 */
function configExample(name: string): string {
  return [
    '# 全局 ~/.gitconfig（每台机器各配一次），以 UnityYAMLMerge 为例：',
    `[merge "${name}"]`,
    '    name = Unity SmartMerge',
    "    driver = '<UnityYAMLMerge.exe 的完整路径>' merge -h -p --force --fallback none %O %B %A %A",
  ].join('\n')
}

/**
 * 仓库顶部的 merge driver 横幅（spec §4.6「结果怎么显示」）。
 *
 * 还没读到（null）和全部正常都不挂；检查失败挂一条 info，**不能显示成配置正常**。
 */
export function driverBanners(report: DriverReport | null): DriverBanner[] {
  if (!report) return []
  const banners: DriverBanner[] = []
  for (const f of report.findings) {
    switch (f.state.kind) {
      case 'ok':
        break
      case 'missing': {
        const who = f.samplePaths[0] ? `${f.samplePaths[0]} 等文件` : '一些文件'
        banners.push({
          tone: 'danger',
          text: `.gitattributes 让 ${who}用 merge driver ${f.name}，但本机 git 配置里没有定义它。`
            + '在这台机器上合并时，git 会往这些文件里写冲突标记，Unity 场景会被写坏。',
          example: configExample(f.name),
        })
        break
      }
      case 'incomplete':
        banners.push({
          tone: 'danger',
          text: `merge driver ${f.name} 只配了一部分，缺少 driver 命令。在这台机器上，所有涉及这类文件的合并都会直接失败。`,
        })
        break
      case 'programMissing':
        banners.push({
          tone: 'warning',
          text: `merge driver ${f.name} 指向的程序不存在：${f.state.program}。合并时它会失败，冲突需要手动解决。`,
        })
        break
    }
  }
  if (report.incomplete) {
    banners.push({ tone: 'info', text: `merge driver 检查未完成：${report.incomplete}` })
  }
  return banners
}

/** 与 `ConfirmRequest` 同名的那几个字段，组件直接展开（engine 不依赖 components）。 */
export interface MergeRiskConfirm {
  title: string
  message: string
  warning: string
  confirmLabel: string
  danger: true
  /** 默认按钮是「取消」（spec §4.6）：回车不能误确认。 */
  focusCancel: true
}

/**
 * 应用发起合并之前的确认（spec §4.6「什么时候检查」）。
 *
 * 只拦「完全没配」：那种情况 git 会悄悄写坏场景文件。配置不完整时 git 自己会 fatal、
 * 什么都不改；程序不存在只是自动合并失效。这两种不拦，横幅已经说明了。
 */
export function mergeRiskConfirm(report: DriverReport): MergeRiskConfirm | null {
  const missing = report.findings.filter((f) => f.state.kind === 'missing').map((f) => f.name)
  if (missing.length === 0) return null
  return {
    title: '合并前确认',
    message: `本机 git 配置里没有定义 merge driver ${missing.join('、')}，.gitattributes 却要求用它合并。`,
    warning: 'git 会退回逐行文本合并，往这些文件里写冲突标记，Unity 场景会被写坏。建议先按顶部横幅的示例配好 driver 再合并。',
    confirmLabel: '仍然继续',
    danger: true,
    focusCancel: true,
  }
}

/**
 * 这条命令会不会让 git 跑合并（从而调用 merge driver）。
 *
 * Task 10 实测：除了 merge / rebase / cherry-pick / revert，`stash apply` 也会调用 driver。
 * SVN 的更新与交付不走这里，由各自的流程调用 `guardMerge`。
 */
const MERGE_COMMANDS = new Set(['pull', 'cherry_pick', 'revert', 'stash_apply', 'stash_pop'])

export function triggersMerge(command: string): boolean {
  return MERGE_COMMANDS.has(command)
}
