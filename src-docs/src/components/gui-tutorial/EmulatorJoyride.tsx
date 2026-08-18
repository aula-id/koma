import { Joyride, type Step } from 'react-joyride'

type Props = { portalId: string; target?: string; title: string; description: string }

/** Container-scoped spotlight and instruction card for docs emulator controls. */
export function EmulatorJoyride({ portalId, target, title, description }: Props) {
  const steps: Step[] = target ? [{ content: description, target: `[data-gui-tour="${target}"]`, title }] : []

  return <Joyride
    run={Boolean(target)}
    stepIndex={0}
    steps={steps}
    portalElement={`#${portalId}`}
    options={{
      buttons: [],
      blockTargetInteraction: false,
      disableFocusTrap: true,
      dismissKeyAction: false,
      overlayClickAction: false,
      overlayColor: 'rgb(11 14 20 / 55%)',
      skipBeacon: true,
      skipScroll: true,
      spotlightPadding: 4,
      spotlightRadius: 4,
      textColor: 'var(--color-koma-fg)',
      width: 230,
      zIndex: 20,
    }}
    styles={{
      floater: { filter: 'drop-shadow(0 5px 14px rgb(0 0 0 / 45%))' },
      tooltip: { backgroundColor: 'var(--color-koma-panel)', border: '1px solid var(--color-koma-border)', borderRadius: 6, fontFamily: 'inherit', fontSize: 11, padding: 10 },
      tooltipContent: { color: 'var(--color-koma-fg)', padding: '6px 0 0', textAlign: 'left' },
      tooltipTitle: { color: 'var(--color-koma-accent)', fontSize: 11, margin: 0, textAlign: 'left' },
    }}
  />
}
