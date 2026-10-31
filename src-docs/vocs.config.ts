import { defineConfig } from 'vocs/config'

const start = [
  ['What koma does for you', '/start/what-koma-does'],
  ['Install koma', '/start/install'],
  ['Open koma', '/start/open'],
  ['Choose how to connect', '/start/welcome'],
  ['Sign in to an account', '/start/sign-in'],
  ['Use your own key', '/start/own-key'],
  ['The chat screen', '/start/chat-screen'],
  ['Send your first request', '/start/first-request'],
  ['Stop koma while it works', '/start/stop'],
  ['Leave and come back', '/start/leave'],
] as const

const conversations = [
  ['What a conversation is', '/conversations/what-is-a-conversation'],
  ['Start another and keep this one', '/conversations/start-another'],
  ['Close this one and start fresh', '/conversations/close-and-start'],
  ['Find a conversation', '/conversations/find'],
  ['Rename a conversation', '/conversations/rename'],
  ['Choose the folder', '/conversations/folders'],
] as const

const asking = [
  ['Write a request', '/asking/write'],
  ['Send the same request again', '/asking/send-again'],
  ['Change an earlier request', '/asking/change-earlier'],
  ['Add a picture or a note', '/asking/pictures'],
  ['Point at a file', '/asking/point-at-a-file'],
  ['While koma is working', '/asking/while-it-works'],
  ['The list koma keeps', '/asking/task-list'],
  ['Commands on your computer', '/asking/commands'],
  ['Let koma look things up', '/asking/internet'],
  ['A conversation that got too long', '/asking/long-conversation'],
] as const

const careful = [
  ['How careful koma is', '/careful/ways-of-working'],
  ['Change how careful it is', '/careful/change-it'],
  ['How hard it thinks', '/careful/effort'],
  ['The free connection', '/careful/free'],
  ['Which model answers', '/careful/which-model'],
] as const

const account = [
  ['A tour of settings', '/account/settings-tour'],
  ['Themes', '/account/themes'],
  ['General settings', '/account/general'],
  ['Providers', '/account/providers'],
  ['Sign-in connections', '/account/sign-in'],
  ['Models', '/account/models'],
  ['Notices', '/account/notices'],
  ['What this conversation cost', '/account/cost'],
] as const

const more = [
  ['Saved ways of working', '/more/skills'],
  ['Extra assistants', '/more/assistants'],
  ['Add-ons you already have', '/more/addons'],
  ['The store', '/more/store'],
  ['Plug in another service', '/more/other-services'],
  ['Work on another computer', '/more/another-computer'],
  ['Safety checks', '/more/safety'],
] as const

const trouble = [
  ['Help inside koma', '/trouble/help'],
  ['Copy the conversation', '/trouble/copy'],
  ['The window is too narrow', '/trouble/too-narrow'],
  ['Quit', '/trouble/quit'],
  ['The terminal cannot start', '/trouble/cannot-start'],
] as const

const commands = [
  ['/new', '/reference/commands/new'],
  ['/new kill', '/reference/commands/new-kill'],
  ['/new remote', '/reference/commands/new-remote'],
  ['/resume', '/reference/commands/resume'],
  ['/resume remote', '/reference/commands/resume-remote'],
  ['/mode', '/reference/commands/mode'],
  ['/effort', '/reference/commands/effort'],
  ['/free', '/reference/commands/free'],
  ['/internet', '/reference/commands/internet'],
  ['/settings', '/reference/commands/settings'],
  ['/agents', '/reference/commands/agents'],
  ['/mcp', '/reference/commands/mcp'],
  ['/extension', '/reference/commands/extension'],
  ['/extension use', '/reference/commands/extension-use'],
  ['/store', '/reference/commands/store'],
  ['/security', '/reference/commands/security'],
  ['/remote', '/reference/commands/remote'],
  ['/task', '/reference/commands/task'],
  ['/model', '/reference/commands/model'],
  ['/bash', '/reference/commands/bash'],
  ['/todo', '/reference/commands/todo'],
  ['/skill', '/reference/commands/skill'],
  ['/cd', '/reference/commands/cd'],
  ['/adddir', '/reference/commands/adddir'],
  ['/compact', '/reference/commands/compact'],
  ['/clear', '/reference/commands/clear'],
  ['/usage', '/reference/commands/usage'],
  ['/rename', '/reference/commands/rename'],
  ['/select', '/reference/commands/select'],
  ['/notification', '/reference/commands/notification'],
  ['/help', '/reference/commands/help'],
  ['/update', '/reference/commands/update'],
  ['/quit', '/reference/commands/quit'],
] as const

function group(text: string, pages: readonly (readonly [string, string])[]) {
  return { text, items: pages.map(([label, link]) => ({ text: label, link })) }
}

export default defineConfig({
  // Served under koma.run/docs by the koma-landing runner.
  basePath: '/docs',
  title: 'Koma',
  description: 'Learn the Koma terminal from the first launch.',
  renderStrategy: 'full-static',
  topNav: [
    { text: 'Docs', link: '/' },
    { text: 'SDK', link: '/sdk' },
    { text: 'Extensions', link: '/extension' },
    { text: 'koma.run', link: 'https://koma.run', external: true },
  ],
  sidebar: {
    '/': [
      { text: 'Start', link: '/' },
      group('Build', [
        ['SDK', '/sdk'],
        ['Extensions', '/extension'],
      ]),
      group('Start here', start),
      group('Conversations', conversations),
      group('Asking for work', asking),
      group('How careful koma is', careful),
      group('Your account', account),
      group('When you need more', more),
      group('When something looks wrong', trouble),
      group('Reference', [['Keys', '/reference/keys'], ...commands]),
    ],
    '/sdk': [
      { text: 'SDK', link: '/sdk' },
      group('Start here', [
        ['Install', '/sdk/install'],
        ['An empty install', '/sdk/empty-install'],
        ['First program', '/sdk/first-program'],
        ['Come back later', '/sdk/resume'],
        ['Python', '/sdk/python'],
        ['TypeScript', '/sdk/typescript'],
      ]),
      group('Running a session', [
        ['Create and resume', '/sdk/session'],
        ['Turns', '/sdk/turns'],
        ['Approvals', '/sdk/approvals'],
        ['Events', '/sdk/events'],
        ['Configure', '/sdk/configure'],
        ['Errors', '/sdk/errors'],
        ['Limits', '/sdk/limits'],
      ]),
      group('Reference', [
        ['Transport', '/sdk/reference/transport'],
        ['State', '/sdk/reference/state'],
        ['Policy and attachments', '/sdk/reference/policy'],
        ['create', '/sdk/reference/create'],
        ['resume', '/sdk/reference/resume'],
        ['state', '/sdk/reference/state-method'],
        ['submit', '/sdk/reference/submit'],
        ['enqueue', '/sdk/reference/enqueue'],
        ['interrupt', '/sdk/reference/interrupt'],
        ['interrupt_and_submit', '/sdk/reference/interrupt-and-submit'],
        ['approve', '/sdk/reference/approve'],
        ['detach', '/sdk/reference/detach'],
        ['configure', '/sdk/reference/configure'],
      ]),
    ],
    '/extension': [
      { text: 'How to use one', link: '/extension' },
      group('Start here', [
        ['Smallest program', '/extension/first'],
        ['Build and sideload', '/extension/sideload'],
        ['Daemon or oneshot', '/extension/kinds'],
        ['Two directions', '/extension/directions'],
      ]),
      group('Manifest', [
        ['Fields', '/extension/manifest'],
        ['Tools', '/extension/contributes/tools'],
        ['Sub-agents', '/extension/contributes/sub-agents'],
        ['Models', '/extension/contributes/models'],
        ['Panels', '/extension/contributes/panels'],
        ['OAuth providers', '/extension/contributes/oauth'],
        ['TUI screens', '/extension/contributes/tui'],
        ['Events', '/extension/contributes/events'],
      ]),
      group('Grants', [
        ['Deadlock rule', '/extension/deadlock'],
        ['agents:read', '/extension/grants/agents-read'],
        ['agents:orchestrate', '/extension/grants/agents-orchestrate'],
        ['sessions:manage', '/extension/grants/sessions-manage'],
        ['chat:prompt', '/extension/grants/chat-prompt'],
        ['models:invoke', '/extension/grants/models-invoke'],
        ['context:publish', '/extension/grants/context-publish'],
        ['models:contribute', '/extension/grants/models-contribute'],
        ['oauth:contribute', '/extension/grants/oauth-contribute'],
      ]),
      group('Surfaces', [
        ['Panel bridge', '/extension/panel'],
        ['TUI screens', '/extension/tui'],
        ['Delegated OAuth', '/extension/oauth'],
        ['Lifecycle', '/extension/lifecycle'],
      ]),
      group('Samples', [
        ['echo-tool-daemon', '/extension/samples/echo-tool-daemon'],
        ['upper-tool-oneshot', '/extension/samples/upper-tool-oneshot'],
        ['agent-peek-oneshot', '/extension/samples/agent-peek-oneshot'],
        ['event-watcher-daemon', '/extension/samples/event-watcher-daemon'],
        ['orchestrator-daemon', '/extension/samples/orchestrator-daemon'],
        ['fleet-board-daemon', '/extension/samples/fleet-board-daemon'],
        ['oauth-demo-daemon', '/extension/samples/oauth-demo-daemon'],
        ['tui-demo-daemon', '/extension/samples/tui-demo-daemon'],
      ]),
    ],
  },
})
