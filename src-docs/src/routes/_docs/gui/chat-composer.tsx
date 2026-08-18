import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/chat-composer')({
  component: GuiChatComposerPage,
})

function GuiChatComposerPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Chat &amp; Composer</h1>
      <p className="mb-6 text-koma-fg">
        The chat panel is the primary interface for interacting with the koma
        agent. It renders markdown messages, tool call results, and sub-agent
        output in a scrollable transcript.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Composer Controls</h3>
          <p>
            The input area at the bottom provides a multiline text field. Below
            it, a row of controls includes the model picker, reasoning effort
            selector, agent mode toggle, and a send button.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Model &amp; Effort</h3>
          <p>
            The model dropdown lists all configured models. Select one to
            override the session model. The effort selector sets reasoning
            depth (low / medium / high). The mode toggle cycles through
            normal, auto, plan, and yolo.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">File &amp; Image Attachments</h3>
          <p>
            Drag files onto the composer or use the attach button to include
            screenshots. Pasted images from the clipboard are staged
            automatically. File paths typed in the input are resolved as
            context references.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">@ File References</h3>
          <p>
            Type <code className="text-koma-fg">@</code> in the composer to search for
            workspace files. Matching files appear in a dropdown. Select one
            to include its path as a context hint for the agent.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Pending Steer</h3>
          <p>
            When the agent pauses for a decision (permission prompt, question,
            or plan approval), a steer banner appears above the composer.
            Respond inline to continue.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Message Rewind</h3>
          <p>
            Hover over a message and click the rewind icon to restore the
            conversation to that point. Messages after the selected point are
            discarded.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Tool Approvals</h3>
          <p>
            Tool calls requiring permission show an approval banner with
            Accept / Reject buttons. In yolo mode, all tools auto-approve.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Plan Decisions</h3>
          <p>
            In plan mode, the agent proposes a task plan before executing.
            The plan appears as a structured card with approve / reject
            controls.
          </p>
        </div>
      </div>
    </article>
  )
}
