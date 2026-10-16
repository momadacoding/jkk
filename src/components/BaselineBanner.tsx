import { useState } from 'react'
import {
  describeAudit,
  describeEolMigration,
  describeSync,
  needsStandaloneCheck,
  syncBannerIsSuppressed,
  type EolStatus,
  type ServerAudit,
  type SyncStatus,
} from '../engine/svnSync'
import {
  describeUnfinishedSync,
  otherBannersAreSuppressed,
  type SyncState,
  type SyncStateUnreadable,
} from '../engine/svnJournal'
import type { SyncActionId } from '../engine/svnSyncIn'
import type { SyncOutActionId } from '../engine/svnSyncOut'
import type { BannerAction } from '../engine/svnSync'

/** 交付那一组按钮的 id。`BannerAction.id` 是所有横幅共用的联合类型，在这里收窄。 */
function isSyncOutAction(id: BannerAction['id']): id is SyncOutActionId {
  return id === 'syncOutCleanup' || id === 'syncOutContinue' || id === 'syncOutAbort'
}

interface BaselineBannerProps {
  /** 是不是 SVN 混合仓库。普通 git 仓库下整个组件什么都不渲染。 */
  isSvn: boolean
  sync: SyncStatus | null
  audit: ServerAudit | null
  /** 行尾规则装了没有（spec §7.3.0）。`null` = 还没读到。 */
  eol: EolStatus | null
  /**
   * 同步锁与上次未收尾的 journal（spec §5.1、§7.6）。`null` = 还没读到；
   * `{ readError }` = 读失败，照样挂红条。
   *
   * 它挂出来的横幅**排在最前，且其余横幅全部让位**——理由见组件里的注释。
   */
  syncState: SyncState | SyncStateUnreadable | null
  /** 重建基线。调用方负责先弹确认。 */
  onRebuild: () => void
  /** 手动体检当前基线。 */
  onCheck: () => void
  /** 走一次行尾迁移。调用方负责先弹确认。 */
  onMigrateEol: () => void
  /** 未收尾同步横幅上「更新」那一组的「继续 / 放弃」。 */
  onSyncAction: (id: SyncActionId) => void
  /** 未收尾同步横幅上「交付」那一组的「清理 / 继续 / 停止」。 */
  onSyncOutAction: (id: SyncOutActionId) => void
  /** 「工作副本已在外部更新」横幅上的「更新…」，与工具栏的「更新」是同一个入口。 */
  onSvnUpdate: () => void
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
 *
 * # 体检入口必须**常驻**，不能只在失同步时出现
 *
 * revision 一致不代表基线内容正确：`Lying`（基线记的内容与服务器不符）恰恰发生在
 * revision 完全对得上的时候。把入口绑在失同步横幅里，等于只在不需要它的时候
 * 提供它（2026-09-16 独立 review）。判据下沉到 `needsStandaloneCheck`，
 * 保证任何时刻**恰好一个**体检入口。
 *
 * # 行尾迁移横幅会让另外两条让位
 *
 * 迁移未完成时基线的**形状**就是错的（spec §7.3.0），此时失同步横幅和常驻体检
 * 入口给出的结论都不可靠，而且三者的出口都指向重建基线——挂三条只是噪音。
 * 判据在 `syncBannerIsSuppressed` / `needsStandaloneCheck` 里，不散进这里。
 *
 * 唯一的例外是 `pinnedReset`：那时 `describeEolMigration` 自己返回 `null`，
 * 失同步横幅照常渲染，因为它指的是另一件必须先做的事。
 */
export function BaselineBanner({
  isSvn,
  sync,
  audit,
  eol,
  syncState,
  onRebuild,
  onCheck,
  onMigrateEol,
  onSyncAction,
  onSyncOutAction,
  onSvnUpdate,
}: BaselineBannerProps) {
  // 未收尾 / 同步进行中排在最前，其余一律让位：那两种状态下仓库停在中间态，
  // 别的横幅拿半推进的基线算出来的结论可能是错的，而它们的出口（重建基线）
  // 此刻是危险的——会移动正被依赖的基线指针。
  const unfinished = describeUnfinishedSync(syncState)
  const hushed = otherBannersAreSuppressed(syncState)
  const eolModel = hushed ? null : describeEolMigration(eol, sync)
  const model = hushed || syncBannerIsSuppressed(eol, sync) ? null : describeSync(sync)
  const auditModel = hushed ? null : describeAudit(audit)
  const standalone = !hushed && needsStandaloneCheck(isSvn, sync, eol)
  const [copied, setCopied] = useState(false)

  if (!unfinished && !eolModel && !model && !auditModel && !standalone) return null

  return (
    <>
      {unfinished && (
        <div className={`banner-baseline ${unfinished.tone}`}>
          <div className="banner-baseline-head">
            <strong>{unfinished.title}</strong>
          </div>
          <p className="banner-baseline-detail">{unfinished.detail}</p>
          {unfinished.note && (
            <p className="banner-baseline-note">{unfinished.note}</p>
          )}
          {unfinished.actions.length > 0 && (
            <div className="banner-baseline-actions">
              {unfinished.actions.map((a) => (
                // 按 id 分流：交付记录的按钮交给「更新」的继续 / 放弃，后端会以
                // sync_out_not_supported 拒绝。其余只可能是 `syncActions` 的那两个值
                // （Task 9 的测试钉着）
                <button
                  key={a.id}
                  onClick={() =>
                    isSyncOutAction(a.id) ? onSyncOutAction(a.id) : onSyncAction(a.id as SyncActionId)
                  }
                >
                  {a.label}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {eolModel && (
        <div className={`banner-baseline ${eolModel.tone}`}>
          <div className="banner-baseline-head">
            <strong>{eolModel.title}</strong>
          </div>
          <p className="banner-baseline-detail">{eolModel.detail}</p>
          {eolModel.note && <p className="banner-baseline-note">{eolModel.note}</p>}
          <div className="banner-baseline-actions">
            {eolModel.actions.map((a) => (
              <button key={a.id} onClick={onMigrateEol}>
                {a.label}
              </button>
            ))}
          </div>
        </div>
      )}
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
                  else if (a.id === 'svnUpdate') onSvnUpdate()
                  else if (a.id === 'updateWhitelistDir' && model.command) {
                    void navigator.clipboard.writeText(model.command)
                    setCopied(true)
                    // 复位，否则按钮永远停在「已复制」，用户再点一次
                    // 不知道到底复制成功没有。
                    window.setTimeout(() => setCopied(false), 1500)
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
      {standalone && (
        <div className="banner-baseline quiet">
          <p className="banner-baseline-detail">
            revision 对得上不代表基线记的内容是对的——体检会拿基线跟服务器逐个文件比一遍。
          </p>
          <div className="banner-baseline-actions">
            <button onClick={onCheck}>体检基线</button>
          </div>
        </div>
      )}
    </>
  )
}

export default BaselineBanner
