import { useState } from 'react'
import {
  WIZARD_STEPS,
  describeGitignoreConflicts,
  describeGitignoreOutcome,
  describeHealth,
  describePinned,
  describeTimings,
  gitignoreBlockReason,
  needsReconfirm,
  nextStep,
  parseExtensions,
  splitErrorMessage,
  stepStatus,
  wizardErrorHint,
} from '../engine/svnInit'
import type { InitPreflight, InitReport, Notice, StepId } from '../engine/svnInit'
import { toErrorCode, toErrorMessage } from '../engine/errors'

interface SvnInitWizardProps {
  repoPath: string
  /** 用户点「取消」：还没建库，什么都不做。 */
  onCancel: () => void
  /**
   * 用户在**完成页**点「关闭」：这时才把仓库重新打开。
   *
   * ⚠️ 不能在 `onInit` 返回前就重开。第一版是那么写的，结果
   * `openRepo` 的第一条语句同步清掉 `errorCode`，RepoTab 立刻把向导卸载掉，
   * `setStep('done')` 落在一个已卸载的组件上——**完成页一帧都没画出来**，
   * 连带 spec §6.8.6 要求显示的「基线未经体检」和 §6.5.3 的 pinned 说明
   * 一起变成死代码。窗口验证时看到「向导退场、主界面接上」，还把它当成了预期行为。
   */
  onFinish: () => void
  /** P1–P4 加起作用的 `.gitignore`，什么都不创建。失败时应当抛出。 */
  onPreflight: (whitelist: string[], extensions: string[]) => Promise<InitPreflight>
  /**
   * 真正建库。失败时应当抛出——向导据此留在原地并显示错误，好让用户修完重试。
   *
   * `acknowledged` 是第 3 步展示过的 `.gitignore`（相对路径），后端拿它与此刻查到的对账。
   */
  onInit: (
    whitelist: string[],
    extensions: string[],
    respectGitignore: boolean,
    acknowledged: string[],
  ) => Promise<InitReport>
}

/** 一条提示的渲染。tone 决定配色，内容全部由 `engine/svnInit` 算好。 */
function NoticeBox({ notice }: { notice: Notice }) {
  return (
    <div className={`dlg-note svn-notice ${notice.tone}`}>
      <strong>{notice.title}</strong>
      <pre className="svn-notice-detail">{notice.detail}</pre>
    </div>
  )
}

/**
 * SVN 混合仓库的初始化向导（spec §6.5）。
 *
 * 组件只做渲染与事件绑定：步骤推进、输入解析、结果与错误的显示分类**全部**在
 * `engine/svnInit.ts` 里，这里不写业务判断。1a 的 P2-2 已经证明「判断散在组件里」
 * 会漏——同一个 `confirm()` 的两半，两轮 review 才抓齐（AGENTS.md §7.2）。
 */
export default function SvnInitWizard({
  repoPath,
  onCancel,
  onFinish,
  onPreflight,
  onInit,
}: SvnInitWizardProps) {
  const [step, setStep] = useState<StepId>('choose')
  const [whitelistText, setWhitelistText] = useState('')
  const [extensionsText, setExtensionsText] = useState('.cs')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<{ code: string; message: string } | null>(null)
  const [preflight, setPreflight] = useState<InitPreflight | null>(null)
  const [report, setReport] = useState<InitReport | null>(null)
  /** 「尊重已有的 .gitignore 规则」，默认不勾（用户 2026-10-28 决定）。 */
  const [respectGitignore, setRespectGitignore] = useState(false)

  const whitelist = whitelistText
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
  const extensions = parseExtensions(extensionsText)

  async function run(fn: () => Promise<void>) {
    setBusy(true)
    setError(null)
    try {
      await fn()
    } catch (e) {
      // `toErrorCode` 对拿不到 code 的异常返回 null；这里统一成空串，
      // 让 `wizardErrorHint` 走「没有对应条目」那条路而不是崩在类型上。
      setError({ code: toErrorCode(e) ?? '', message: toErrorMessage(e) })
    } finally {
      setBusy(false)
    }
  }

  const conflicts = preflight?.gitignoreConflicts ?? []
  const conflictsNotice = describeGitignoreConflicts(conflicts)
  const blockReason = gitignoreBlockReason(conflicts, respectGitignore)

  const healthNotice = report ? describeHealth(report.health) : null
  const pinnedNotice = report ? describePinned(report.pinned) : null
  const timingsNotice = report ? describeTimings(report.timings) : null
  const gitignoreNotice = report ? describeGitignoreOutcome(report.gitignore) : null
  const errorParts = error ? splitErrorMessage(error.message) : null
  const errorHint = error ? wizardErrorHint(error.code, error.message) : ''

  return (
    <div className="dlg svn-wizard">
      <div className="dlg-head">
        <div>
          <h3>把这个 SVN 工作副本交给 git 管</h3>
          <div className="dlg-value">{repoPath}</div>
        </div>
      </div>

      <ol className="svn-steps">
        {WIZARD_STEPS.map((s) => (
          <li key={s.id} className={`svn-step ${stepStatus(s.id, step)}`}>
            <span className="svn-step-title">{s.title}</span>
            <span className="svn-step-hint">{s.hint}</span>
          </li>
        ))}
      </ol>

      {step === 'choose' && (
        <>
          <div className="dlg-field">
            <label className="dlg-field-label" htmlFor="svn-whitelist">
              交给 git 管的目录（一行一条）
            </label>
            <textarea
              id="svn-whitelist"
              rows={4}
              value={whitelistText}
              onChange={(e) => setWhitelistText(e.target.value)}
              placeholder={'Assets/Scripts/\nAssets/Editor/'}
            />
          </div>
          <div className="dlg-field">
            <label className="dlg-field-label" htmlFor="svn-ext">
              收哪些扩展名（逗号分隔，`*` 表示全收）
            </label>
            <input
              id="svn-ext"
              value={extensionsText}
              onChange={(e) => setExtensionsText(e.target.value)}
            />
            <div className="dlg-note">实际写入配置的是：{extensions.join(' ') || '（空）'}</div>
          </div>
        </>
      )}

      {step === 'preflight' && preflight && (
        <div className="dlg-note">
          <strong>检查通过。</strong>
          <div>记账 revision：r{preflight.nodeRev}</div>
          <div>仓库根：{preflight.reposRoot}</div>
          <div>仓库 UUID：{preflight.uuid}</div>
        </div>
      )}

      {/* 起作用的 .gitignore 列在「建库」按钮上方；没有冲突时这一块和勾选框都不渲染 */}
      {step === 'confirm' && conflictsNotice && (
        <>
          <NoticeBox notice={conflictsNotice} />
          {/* `.dlg-field` 是横向 flex：说明文字放进去会被挤到右边一列，所以单独成块 */}
          <label className="dlg-field dlg-check">
            <input
              type="checkbox"
              checked={respectGitignore}
              onChange={(e) => setRespectGitignore(e.target.checked)}
              disabled={busy}
            />
            尊重已有的 .gitignore 规则
          </label>
          <div className="dlg-note">
            不勾：上面这些 .gitignore 会改名为 .gitignore.jkk-disabled 停用，文件照样交给 git 管。
            <br />
            勾上：被它们挡掉的文件不交给 git 管，以后的更新、交付也按这个口径。
          </div>
          {blockReason && <div className="dlg-note svn-notice error">{blockReason}</div>}
        </>
      )}

      {step === 'done' && report && (
        <>
          <div className="dlg-note">
            <strong>建好了。</strong>
            <div>首个基线：r{report.nodeRev}（{report.baseline.slice(0, 10)}）</div>
            <div>当前分支已切到 work，基线分支是 svn/base。</div>
          </div>
          {gitignoreNotice && <NoticeBox notice={gitignoreNotice} />}
          {healthNotice && <NoticeBox notice={healthNotice} />}
          {pinnedNotice && <NoticeBox notice={pinnedNotice} />}
          {timingsNotice && <NoticeBox notice={timingsNotice} />}
        </>
      )}

      {errorParts && (
        <div className="dlg-note svn-notice error">
          <strong>{errorParts.headline}</strong>
          {/* 多行详情（分组报告、耗时）必须保留换行，而且可能有几十行——
              放进可滚动的 <pre>，不能把对话框撑出屏幕 */}
          {errorParts.detail && (
            <pre className="svn-notice-detail svn-notice-scroll">{errorParts.detail}</pre>
          )}
          {/* 空串时不渲染这一段——`wizardErrorHint` 对未知错误码返回 ''，
              直接渲染会变成一个空的提示块 */}
          {errorHint && <pre className="svn-notice-detail">{errorHint}</pre>}
        </div>
      )}

      <div className="dlg-actions">
        {step !== 'done' && (
          <button type="button" onClick={onCancel} disabled={busy}>
            取消
          </button>
        )}
        {step === 'choose' && (
          <button
            type="button"
            disabled={busy || whitelist.length === 0 || extensions.length === 0}
            onClick={() =>
              run(async () => {
                setPreflight(await onPreflight(whitelist, extensions))
                setStep(nextStep('choose') ?? 'preflight')
              })
            }
          >
            检查工作副本
          </button>
        )}
        {step === 'preflight' && (
          <button type="button" disabled={busy} onClick={() => setStep('confirm')}>
            下一步
          </button>
        )}
        {step === 'confirm' && (
          <button
            type="button"
            disabled={busy || blockReason !== null}
            onClick={() =>
              run(async () => {
                try {
                  setReport(
                    await onInit(
                      whitelist,
                      extensions,
                      respectGitignore,
                      conflicts.map((c) => c.path),
                    ),
                  )
                  setStep('done')
                } catch (e) {
                  // 出现了用户没看过的 .gitignore：重新检查、把新清单摆出来，留在这一步。
                  // 错误照常显示，告诉用户为什么清单变了
                  if (needsReconfirm(toErrorCode(e) ?? '')) {
                    setPreflight(await onPreflight(whitelist, extensions))
                  }
                  throw e
                }
              })
            }
          >
            建库
          </button>
        )}
        {step === 'done' && (
          <button type="button" onClick={onFinish}>
            关闭
          </button>
        )}
      </div>
    </div>
  )
}
