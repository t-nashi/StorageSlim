export function AppHeader({
  iconUrl,
  tagline,
  version,
}: {
  iconUrl: string;
  tagline: string;
  version: string | null;
}) {
  return (
    <section className="app-header panel">
      <div className="app-identity">
        {/* 見出しが製品名を読み上げるので、アイコンは装飾として alt を空にする */}
        <img className="app-logo" src={iconUrl} alt="" width={56} height={56} />
        <div className="app-identity-text">
          <h1>StorageSlim</h1>
          <p className="app-tagline">{tagline}</p>
        </div>
      </div>
      {version ? <span className="app-version">v{version}</span> : null}
    </section>
  );
}
