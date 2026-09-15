import { useState } from 'react'
import {
  describeAudit,
  describeSync,
  type ServerAudit,
  type SyncStatus,
} from '../engine/svnSync'

interface BaselineBannerProps {
  sync: SyncStatus | null
  audit: ServerAudit | null
  /** 重建基线。调用方负责先弹确认。 */
  onRebuild: () => void
  /** 手动体检当前基线。 */
  onCheck: () => void
}

/**
 * 失同步横幅 + 服务器侧漏收提示（spec §6.6、§6.3.1）。
 *
 * # 这里也是「重建基线」与「手动体检」唯一的入口
 *
 * 两个后端命令 Phase 1b 就做好了，但一直没有界面入口（1b 决策日志 NB-6）。
 * spec §16 的 Phase 1 验收语写着「此期只报警 + **提供重建基线**」——没有入口，
 * 那一期是不自洽的：基线一旦被外部 TortoiseSVN 操作弄失同步，
 * 「相对上次更新改了什么」这个核心收益就永久失效、且没有任何出路。
 *
 * # 两条提示是**独立**的，不能互相覆盖
 *
 * 失同步说的是「revision 对不上」，漏收说的是「文件根本没下来」。
 * 一个正常、另一个不正常是完全可能的组合，所以分两条渲染。
 */
export function BaselineBanner({ sync, audit, onRebuild, onCheck }: BaselineBannerProps) {
  const model = describeSync(sync)
  const auditModel = describeAudit(audit)
  const [copied, setCopied] = useState(false)

  if (!model && !auditModel) return null

  return (
    <>
      {model && (
        <div className={`banner-baseline ${model.tone}`}>
          <div className="banner-baseline-head">
            <strong>{model.title}</strong>
          </div>
          <p className="banner-baseline-detail">{model.detail}</p>
          {model.command && (
            <pre className="banner-baseline-command">{model.command}</pre>
          )}
          {model.note && <p className="banner-baseline-note">{model.note}</p>}
          <div className="banner-baseline-actions">
            {model.actions.map((a) => (
              <button
                key={a.id}
                onClick={() => {
                  if (a.id === 'rebuild') onRebuild()
                  else if (a.id === 'check') onCheck()
                  else if (a.id === 'updateWhitelistDir' && model.command) {
                    void navigator.clipboard.writeText(model.command)
                    setCopied(true)
                  }
                }}
              >
                {a.id === 'updateWhitelistDir' && copied ? '已复制' : a.label}
              </button>
            ))}
          </div>
        </div>
      )}
      {auditModel && (
        <div className={`banner-baseline ${auditModel.tone}`}>
          <div className="banner-baseline-head">
            <strong>{auditModel.title}</strong>
          </div>
          <p className="banner-baseline-detail">{auditModel.detail}</p>
          <ul className="banner-baseline-files">
            {auditModel.files.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
        </div>
      )}
    </>
  )
}

export default BaselineBanner
