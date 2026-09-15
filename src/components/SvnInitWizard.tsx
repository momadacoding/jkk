import { useState } from 'react'
import {
  WIZARD_STEPS,
  describeHealth,
  describePinned,
  nextStep,
  parseExtensions,
  stepStatus,
  wizardErrorHint,
} from '../engine/svnInit'
import type { InitReport, Notice, Preflight, StepId } from '../engine/svnInit'
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
  /** 只跑 P1–P4，什么都不创建。失败时应当抛出。 */
  onPreflight: (whitelist: string[]) => Promise<Preflight>
  /** 真正建库。失败时应当抛出——向导据此留在原地并显示错误，好让用户修完重试。 */
  onInit: (whitelist: string[], extensions: string[]) => Promise<InitReport>
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
  const [preflight, setPreflight] = useState<Preflight | null>(null)
  const [report, setReport] = useState<InitReport | null>(null)

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

  const healthNotice = report ? describeHealth(report.health) : null
  const pinnedNotice = report ? describePinned(report.pinned) : null

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

      {step === 'done' && report && (
        <>
          <div className="dlg-note">
            <strong>建好了。</strong>
            <div>首个基线：r{report.nodeRev}（{report.baseline.slice(0, 10)}）</div>
            <div>当前分支已切到 work，基线分支是 svn/base。</div>
          </div>
          {healthNotice && <NoticeBox notice={healthNotice} />}
          {pinnedNotice && <NoticeBox notice={pinnedNotice} />}
        </>
      )}

      {error && (
        <div className="dlg-note svn-notice error">
          <strong>{error.message}</strong>
          {/* 空串时不渲染这一段——`wizardErrorHint` 对未知错误码返回 ''，
              直接渲染会变成一个空的提示块 */}
          {wizardErrorHint(error.code, error.message) && (
            <pre className="svn-notice-detail">
              {wizardErrorHint(error.code, error.message)}
            </pre>
          )}
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
                setPreflight(await onPreflight(whitelist))
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
            disabled={busy}
            onClick={() =>
              run(async () => {
                setReport(await onInit(whitelist, extensions))
                setStep('done')
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
