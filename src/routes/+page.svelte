<script lang="ts">
  type NavItem = {
    label: string;
    description: string;
    active?: boolean;
  };

  const navItems: NavItem[] = [
    {
      label: "Projects",
      description: "Imported Compose workspaces",
      active: true,
    },
    {
      label: "Reports",
      description: "Analysis and review history",
    },
    {
      label: "Engines",
      description: "Docker-compatible runtimes",
    },
    {
      label: "Settings",
      description: "Studio and daemon preferences",
    },
  ];

  const projectRows = [
    { name: "No projects imported", path: "Connect the daemon to add a workspace" },
  ];
</script>

<svelte:head>
  <title>Susun Studio</title>
</svelte:head>

<div class="app-shell">
  <aside class="sidebar" aria-label="Primary">
    <div class="brand">
      <div class="brand-mark">S</div>
      <div>
        <h1>Susun Studio</h1>
        <p>Daemon-first Compose workspace</p>
      </div>
    </div>

    <nav class="nav-list">
      {#each navItems as item}
        <button class:active={item.active} type="button">
          <span>{item.label}</span>
          <small>{item.description}</small>
        </button>
      {/each}
    </nav>

    <div class="daemon-card">
      <span class="status-dot" aria-hidden="true"></span>
      <div>
        <strong>Daemon disconnected</strong>
        <p>Phase 1 will connect through the local authenticated API.</p>
      </div>
    </div>
  </aside>

  <main class="content">
    <header class="topbar">
      <div>
        <p class="eyebrow">Phase 1 Foundation</p>
        <h2>Projects</h2>
      </div>
      <button class="primary-action" type="button">Import Project</button>
    </header>

    <section class="hero-panel" aria-labelledby="daemon-heading">
      <div>
        <p class="eyebrow">Local platform spine</p>
        <h3 id="daemon-heading">Connect to Susun Studio daemon to begin</h3>
        <p>
          The desktop app is only the client. Workspaces, projects, settings,
          events, and future engine tasks live behind the local daemon API.
        </p>
      </div>
      <div class="health-box">
        <span>Health</span>
        <strong>Waiting</strong>
      </div>
    </section>

    <section class="workspace-grid" aria-label="Project overview">
      <div class="section-heading">
        <h3>Workspace Projects</h3>
        <p>Persisted projects will appear here after the daemon API is wired.</p>
      </div>

      <div class="table-shell">
        <div class="table-row table-head">
          <span>Name</span>
          <span>Path</span>
        </div>
        {#each projectRows as project}
          <div class="table-row muted">
            <span>{project.name}</span>
            <span>{project.path}</span>
          </div>
        {/each}
      </div>
    </section>
  </main>
</div>

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(html),
  :global(body) {
    margin: 0;
    min-width: 320px;
    min-height: 100vh;
    color: #172026;
    background: #eef2f3;
    font-family:
      Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  }

  :global(button) {
    font: inherit;
  }

  .app-shell {
    display: grid;
    grid-template-columns: 280px minmax(0, 1fr);
    min-height: 100vh;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding: 22px 16px;
    border-right: 1px solid #d5dddf;
    background: #fbfcfc;
  }

  .brand {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 4px 6px;
  }

  .brand-mark {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border-radius: 8px;
    color: #fff;
    background: #0f766e;
    font-weight: 700;
  }

  h1,
  h2,
  h3,
  p {
    margin: 0;
  }

  .brand h1 {
    font-size: 17px;
    line-height: 1.2;
  }

  .brand p,
  .daemon-card p,
  .section-heading p,
  .hero-panel p,
  .nav-list small {
    color: #5b6870;
  }

  .brand p {
    margin-top: 2px;
    font-size: 12px;
  }

  .nav-list {
    display: grid;
    gap: 6px;
  }

  .nav-list button {
    display: grid;
    gap: 3px;
    width: 100%;
    padding: 10px 12px;
    border: 1px solid transparent;
    border-radius: 8px;
    text-align: left;
    color: #223039;
    background: transparent;
    cursor: pointer;
  }

  .nav-list button:hover,
  .nav-list button.active {
    border-color: #c6d8d6;
    background: #edf7f5;
  }

  .nav-list span {
    font-weight: 650;
  }

  .nav-list small {
    font-size: 12px;
    line-height: 1.35;
  }

  .daemon-card {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin-top: auto;
    padding: 12px;
    border: 1px solid #d5dddf;
    border-radius: 8px;
    background: #fff;
  }

  .status-dot {
    width: 9px;
    height: 9px;
    margin-top: 5px;
    border-radius: 999px;
    background: #c2410c;
  }

  .daemon-card strong {
    display: block;
    margin-bottom: 3px;
    font-size: 13px;
  }

  .daemon-card p {
    font-size: 12px;
    line-height: 1.4;
  }

  .content {
    display: grid;
    align-content: start;
    gap: 20px;
    padding: 24px;
  }

  .topbar,
  .hero-panel,
  .workspace-grid {
    width: min(100%, 1120px);
  }

  .topbar {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    align-items: center;
  }

  .eyebrow {
    color: #0f766e;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0;
    text-transform: uppercase;
  }

  .topbar h2 {
    margin-top: 3px;
    font-size: 28px;
    line-height: 1.1;
  }

  .primary-action {
    min-height: 36px;
    padding: 0 14px;
    border: 1px solid #0f766e;
    border-radius: 8px;
    color: #fff;
    background: #0f766e;
  }

  .hero-panel {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 180px;
    gap: 20px;
    align-items: center;
    padding: 22px;
    border: 1px solid #d5dddf;
    border-radius: 8px;
    background: #fff;
  }

  .hero-panel h3 {
    margin: 6px 0 8px;
    font-size: 24px;
    line-height: 1.2;
  }

  .hero-panel p {
    max-width: 680px;
    line-height: 1.55;
  }

  .health-box {
    display: grid;
    gap: 4px;
    padding: 16px;
    border-radius: 8px;
    background: #f3f6f6;
  }

  .health-box span {
    color: #5b6870;
    font-size: 12px;
  }

  .health-box strong {
    font-size: 20px;
  }

  .workspace-grid {
    display: grid;
    gap: 12px;
  }

  .section-heading {
    display: grid;
    gap: 4px;
  }

  .section-heading h3 {
    font-size: 18px;
  }

  .table-shell {
    overflow: hidden;
    border: 1px solid #d5dddf;
    border-radius: 8px;
    background: #fff;
  }

  .table-row {
    display: grid;
    grid-template-columns: minmax(180px, 0.7fr) minmax(0, 1.3fr);
    gap: 16px;
    padding: 12px 14px;
    border-top: 1px solid #e7ecee;
  }

  .table-row:first-child {
    border-top: 0;
  }

  .table-head {
    color: #52616a;
    background: #f7f9f9;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
  }

  .muted {
    color: #5b6870;
  }

  @media (max-width: 760px) {
    .app-shell {
      grid-template-columns: 1fr;
    }

    .sidebar {
      min-height: auto;
      border-right: 0;
      border-bottom: 1px solid #d5dddf;
    }

    .hero-panel,
    .table-row {
      grid-template-columns: 1fr;
    }

    .topbar {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>