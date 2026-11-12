// deno-fmt-ignore-file
// biome-ignore format: generated types do not need formatting
// prettier-ignore
import type { PathsForPages } from 'waku/router'

// prettier-ignore
type Page =
  | { path: '/account/cost'; render: 'static' }
  | { path: '/account/general'; render: 'static' }
  | { path: '/account/models'; render: 'static' }
  | { path: '/account/notices'; render: 'static' }
  | { path: '/account/providers'; render: 'static' }
  | { path: '/account/settings-tour'; render: 'static' }
  | { path: '/account/sign-in'; render: 'static' }
  | { path: '/account/themes'; render: 'static' }
  | { path: '/asking/change-earlier'; render: 'static' }
  | { path: '/asking/commands'; render: 'static' }
  | { path: '/asking/internet'; render: 'static' }
  | { path: '/asking/long-conversation'; render: 'static' }
  | { path: '/asking/pictures'; render: 'static' }
  | { path: '/asking/point-at-a-file'; render: 'static' }
  | { path: '/asking/send-again'; render: 'static' }
  | { path: '/asking/task-list'; render: 'static' }
  | { path: '/asking/while-it-works'; render: 'static' }
  | { path: '/asking/write'; render: 'static' }
  | { path: '/careful/change-it'; render: 'static' }
  | { path: '/careful/effort'; render: 'static' }
  | { path: '/careful/free'; render: 'static' }
  | { path: '/careful/ways-of-working'; render: 'static' }
  | { path: '/careful/which-model'; render: 'static' }
  | { path: '/conversations/close-and-start'; render: 'static' }
  | { path: '/conversations/find'; render: 'static' }
  | { path: '/conversations/folders'; render: 'static' }
  | { path: '/conversations/rename'; render: 'static' }
  | { path: '/conversations/start-another'; render: 'static' }
  | { path: '/conversations/what-is-a-conversation'; render: 'static' }
  | { path: '/extension/contributes/events'; render: 'static' }
  | { path: '/extension/contributes/models'; render: 'static' }
  | { path: '/extension/contributes/oauth'; render: 'static' }
  | { path: '/extension/contributes/panels'; render: 'static' }
  | { path: '/extension/contributes/sub-agents'; render: 'static' }
  | { path: '/extension/contributes/tools'; render: 'static' }
  | { path: '/extension/contributes/tui'; render: 'static' }
  | { path: '/extension/deadlock'; render: 'static' }
  | { path: '/extension/directions'; render: 'static' }
  | { path: '/extension/first'; render: 'static' }
  | { path: '/extension/grants/agents-orchestrate'; render: 'static' }
  | { path: '/extension/grants/agents-read'; render: 'static' }
  | { path: '/extension/grants/chat-prompt'; render: 'static' }
  | { path: '/extension/grants/context-publish'; render: 'static' }
  | { path: '/extension/grants/models-contribute'; render: 'static' }
  | { path: '/extension/grants/models-invoke'; render: 'static' }
  | { path: '/extension/grants/oauth-contribute'; render: 'static' }
  | { path: '/extension/grants/sessions-manage'; render: 'static' }
  | { path: '/extension'; render: 'static' }
  | { path: '/extension/kinds'; render: 'static' }
  | { path: '/extension/lifecycle'; render: 'static' }
  | { path: '/extension/manifest'; render: 'static' }
  | { path: '/extension/oauth'; render: 'static' }
  | { path: '/extension/panel'; render: 'static' }
  | { path: '/extension/samples/agent-peek-oneshot'; render: 'static' }
  | { path: '/extension/samples/echo-tool-daemon'; render: 'static' }
  | { path: '/extension/samples/event-watcher-daemon'; render: 'static' }
  | { path: '/extension/samples/fleet-board-daemon'; render: 'static' }
  | { path: '/extension/samples/oauth-demo-daemon'; render: 'static' }
  | { path: '/extension/samples/orchestrator-daemon'; render: 'static' }
  | { path: '/extension/samples/tui-demo-daemon'; render: 'static' }
  | { path: '/extension/samples/upper-tool-oneshot'; render: 'static' }
  | { path: '/extension/sideload'; render: 'static' }
  | { path: '/extension/tui'; render: 'static' }
  | { path: '/'; render: 'static' }
  | { path: '/more/addons'; render: 'static' }
  | { path: '/more/another-computer'; render: 'static' }
  | { path: '/more/assistants'; render: 'static' }
  | { path: '/more/other-services'; render: 'static' }
  | { path: '/more/safety'; render: 'static' }
  | { path: '/more/skills'; render: 'static' }
  | { path: '/more/store'; render: 'static' }
  | { path: '/reference/commands/adddir'; render: 'static' }
  | { path: '/reference/commands/agents'; render: 'static' }
  | { path: '/reference/commands/bash'; render: 'static' }
  | { path: '/reference/commands/cd'; render: 'static' }
  | { path: '/reference/commands/clear'; render: 'static' }
  | { path: '/reference/commands/compact'; render: 'static' }
  | { path: '/reference/commands/effort'; render: 'static' }
  | { path: '/reference/commands/extension-use'; render: 'static' }
  | { path: '/reference/commands/extension'; render: 'static' }
  | { path: '/reference/commands/free'; render: 'static' }
  | { path: '/reference/commands/help'; render: 'static' }
  | { path: '/reference/commands/internet'; render: 'static' }
  | { path: '/reference/commands/mcp'; render: 'static' }
  | { path: '/reference/commands/mode'; render: 'static' }
  | { path: '/reference/commands/model'; render: 'static' }
  | { path: '/reference/commands/new-kill'; render: 'static' }
  | { path: '/reference/commands/new-remote'; render: 'static' }
  | { path: '/reference/commands/new'; render: 'static' }
  | { path: '/reference/commands/notification'; render: 'static' }
  | { path: '/reference/commands/quit'; render: 'static' }
  | { path: '/reference/commands/remote'; render: 'static' }
  | { path: '/reference/commands/rename'; render: 'static' }
  | { path: '/reference/commands/resume-remote'; render: 'static' }
  | { path: '/reference/commands/resume'; render: 'static' }
  | { path: '/reference/commands/security'; render: 'static' }
  | { path: '/reference/commands/select'; render: 'static' }
  | { path: '/reference/commands/settings'; render: 'static' }
  | { path: '/reference/commands/skill'; render: 'static' }
  | { path: '/reference/commands/store'; render: 'static' }
  | { path: '/reference/commands/task'; render: 'static' }
  | { path: '/reference/commands/todo'; render: 'static' }
  | { path: '/reference/commands/usage'; render: 'static' }
  | { path: '/reference/keys'; render: 'static' }
  | { path: '/sdk/approvals'; render: 'static' }
  | { path: '/sdk/configure'; render: 'static' }
  | { path: '/sdk/empty-install'; render: 'static' }
  | { path: '/sdk/errors'; render: 'static' }
  | { path: '/sdk/events'; render: 'static' }
  | { path: '/sdk/first-program'; render: 'static' }
  | { path: '/sdk'; render: 'static' }
  | { path: '/sdk/install'; render: 'static' }
  | { path: '/sdk/limits'; render: 'static' }
  | { path: '/sdk/python'; render: 'static' }
  | { path: '/sdk/reference/approve'; render: 'static' }
  | { path: '/sdk/reference/configure'; render: 'static' }
  | { path: '/sdk/reference/create'; render: 'static' }
  | { path: '/sdk/reference/detach'; render: 'static' }
  | { path: '/sdk/reference/enqueue'; render: 'static' }
  | { path: '/sdk/reference/interrupt-and-submit'; render: 'static' }
  | { path: '/sdk/reference/interrupt'; render: 'static' }
  | { path: '/sdk/reference/policy'; render: 'static' }
  | { path: '/sdk/reference/resume'; render: 'static' }
  | { path: '/sdk/reference/state-method'; render: 'static' }
  | { path: '/sdk/reference/state'; render: 'static' }
  | { path: '/sdk/reference/submit'; render: 'static' }
  | { path: '/sdk/reference/transport'; render: 'static' }
  | { path: '/sdk/resume'; render: 'static' }
  | { path: '/sdk/session'; render: 'static' }
  | { path: '/sdk/turns'; render: 'static' }
  | { path: '/sdk/typescript'; render: 'static' }
  | { path: '/start/chat-screen'; render: 'static' }
  | { path: '/start/first-request'; render: 'static' }
  | { path: '/start/install'; render: 'static' }
  | { path: '/start/leave'; render: 'static' }
  | { path: '/start/open'; render: 'static' }
  | { path: '/start/own-key'; render: 'static' }
  | { path: '/start/sign-in'; render: 'static' }
  | { path: '/start/stop'; render: 'static' }
  | { path: '/start/welcome'; render: 'static' }
  | { path: '/start/what-koma-does'; render: 'static' }
  | { path: '/trouble/cannot-start'; render: 'static' }
  | { path: '/trouble/copy'; render: 'static' }
  | { path: '/trouble/help'; render: 'static' }
  | { path: '/trouble/quit'; render: 'static' }
  | { path: '/trouble/too-narrow'; render: 'static' }

// prettier-ignore
declare module 'waku/router' {
  interface RouteConfig {
    paths: PathsForPages<Page>
  }
  interface CreatePagesConfig {
    pages: Page
  }
}
