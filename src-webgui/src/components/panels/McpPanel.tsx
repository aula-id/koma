import { useEffect, useState } from "react";
import { McpListView } from "./mcp/McpListView";
import { useKoma } from "../../store/koma";
import { mcpEditTabId } from "../../store/actions/mcp";

// Design reference: master list in the sidebar; the add/edit form and the JSON import
// open as editor tabs (McpTab). `servers` is the authoritative config slice (pushed by
// the host); enable/delete/etc. emit GuiReqs and wait for the host's Config push to
// reflect back, rather than mutating local state optimistically.
export function McpPanel() {
  const servers = useKoma((s) => s.config.mcp);
  const project = useKoma((s) => s.config.mcpProject);
  const req = useKoma((s) => s.req);
  const refreshMcpStatus = useKoma((s) => s.refreshMcpStatus);
  const openMcpTab = useKoma((s) => s.openMcpTab);
  const closeTab = useKoma((s) => s.closeTab);
  const activeTabId = useKoma((s) => s.ui.activeTabId);
  const [armed, setArmed] = useState<string | null>(null);

  // Refresh live connection status each time the MCP sidebar is opened.
  useEffect(() => {
    refreshMcpStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const remove = (id: string) => {
    req({ r: "DeleteMcpServer", uuid: id });
    closeTab(mcpEditTabId(id));
    setArmed(null);
  };
  const toggleEnable = (id: string, enabled: boolean) => {
    req({ r: "EnableMcpServer", uuid: id, enabled });
  };
  const restart = (id: string) => {
    req({ r: "RestartMcpServer", uuid: id });
    setTimeout(refreshMcpStatus, 1500);
  };
  const approve = (id: string, approved: boolean) => {
    req({ r: "ApproveMcpProjectServer", uuid: id, approved });
    setTimeout(refreshMcpStatus, 1500);
  };

  return (
    <div className="relative h-full overflow-hidden">
      <div className="absolute inset-0 flex flex-col bg-koma-panel">
        <McpListView
          servers={servers}
          project={project}
          armed={armed}
          activeTabId={activeTabId}
          onAdd={() => openMcpTab({ mode: "new" })}
          onImport={() => openMcpTab({ mode: "import" })}
          onEdit={(s) => openMcpTab({ mode: "edit", uuid: s.id, title: s.name })}
          onArm={(id) => setArmed(id)}
          onDisarm={() => setArmed(null)}
          onConfirm={remove}
          onToggleEnable={toggleEnable}
          onRestart={restart}
          onApprove={approve}
        />
      </div>
    </div>
  );
}
