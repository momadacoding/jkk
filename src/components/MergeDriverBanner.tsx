import { driverBanners, type DriverReport } from '@/engine/mergeDrivers'

/**
 * 本机 merge driver 配置问题的横幅（spec §4.6）。只提示、给出可以复制的配置，
 * 不替用户改 git 配置。在命令行或 Fork 里发起的合并拦不住，只能靠这里提醒。
 */
export function MergeDriverBanner({ report }: { report: DriverReport | null }) {
  const banners = driverBanners(report)
  if (banners.length === 0) return null
  return (
    <>
      {banners.map((b, i) => (
        <div key={i} className={`banner-driver ${b.tone}`}>
          <div>{b.text}</div>
          {b.example && <pre className="banner-driver-example">{b.example}</pre>}
        </div>
      ))}
    </>
  )
}
