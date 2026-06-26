/**
 * ExcelWorkspace — the "Celnet for Excel" integration page.
 *
 * Explains the Office.js add-in (the `CELNET.*` worksheet functions + ticket task
 * pane that bring Celnet pricing into Excel over the same WS contract as this app),
 * and gives the two get-started actions: download an example workbook and download
 * the add-in manifest to sideload. The downloadable artifacts are served from the
 * GUI's public dir under `/excel/` (copied from the add-in project `excel/`:
 * `manifest.xml` and the example `contribution.xlsx`). The deep reference is the
 * add-in `excel/README.md` and `docs/EXCEL-INTEGRATION.md`.
 */

import styles from "./ExcelWorkspace.module.css";

/** Served (public) paths of the downloadable Excel artifacts. */
const WORKBOOK_URL = "/excel/celnet-fx-options.xlsx";
const MANIFEST_URL = "/excel/manifest.xml";
const WORKBOOK_FILENAME = "celnet-fx-options.xlsx";
const MANIFEST_FILENAME = "celnet-manifest.xml";

/** The default WS endpoint the add-in dials (matches the add-in's transport default). */
const DEFAULT_WS_ENDPOINT = "ws://127.0.0.1:8081";

interface FnRow {
  readonly fn: string;
  readonly what: string;
}

const FUNCTIONS: readonly FnRow[] = [
  { fn: "=CELNET.PRICE(…)", what: "Mid premium for a pair or any instrument token (vanilla → exotic)." },
  { fn: "=CELNET.GREEKS(…)", what: "The 13 Greeks as a labelled spill, with the resolved convention footer." },
  { fn: "=CELNET.RFQ(…)", what: "A firm two-way: bid · offer · quoteId · validUntil." },
  { fn: "=CELNET.SUBSCRIBE(…)", what: "A live, re-ticking, stale-aware streaming two-way." },
  { fn: "=CELNET.SURFACE(pair, tenor)", what: "The smile (delta pillars × vol) + arb / model / convention footer." },
  { fn: "=CELNET.RISK(dim, ccy)", what: "Server-side hierarchical risk rolled up over an org dimension." },
];

const EXAMPLE_FORMULAS = `=CELNET.PRICE("EURUSD","1Y",1.12,"C",1000000)
=CELNET.GREEKS("EURUSD","1Y",1.12,"C")
=CELNET.RFQ("EURUSD","1Y","25dP","C")
=CELNET.SUBSCRIBE("EURUSD","1Y",1.12,"C")
=CELNET.SURFACE("EURUSD","1Y")`;

export function ExcelWorkspace(): React.ReactElement {
  return (
    <div className={styles.root}>
      <div className={styles.inner}>
        <header className={styles.hero}>
          <p className={styles.eyebrow}>Integrations</p>
          <h1 className={styles.title}>Celnet for Excel</h1>
          <p className={styles.lede}>
            Price FX options — and every exotic family, across five asset classes — right
            in your spreadsheet. The Celnet add-in adds a family of <code className={styles.inlineCode}>CELNET.*</code>{" "}
            worksheet functions and a trading task pane that talk to the <strong>same</strong> edge
            this app does, so a cell is <strong>bit-identical</strong> to what you see here, with the
            resolved convention shown alongside every value.
          </p>
        </header>

        <section className={styles.cards}>
          <article className={styles.card}>
            <span className={styles.cardGlyph} aria-hidden>
              ▦
            </span>
            <h2 className={styles.cardTitle}>Example workbook</h2>
            <p className={styles.cardBody}>
              A ready-made workbook wired with live pricing, RFQ, surface and risk formulas across
              the asset classes — open it to see the functions in action.
            </p>
            <div className={styles.cardActions}>
              <a className={`${styles.btn} ${styles.btnPrimary}`} href={WORKBOOK_URL} download={WORKBOOK_FILENAME}>
                ⤓ Download workbook
              </a>
            </div>
          </article>

          <article className={styles.card}>
            <span className={styles.cardGlyph} aria-hidden>
              ⊞
            </span>
            <h2 className={styles.cardTitle}>Set up the add-in</h2>
            <p className={styles.cardBody}>
              Download the add-in manifest and sideload it into Excel (web or desktop) to register
              the <code className={styles.inlineCode}>CELNET.*</code> functions and the Celnet ticket
              task pane. Steps are below.
            </p>
            <div className={styles.cardActions}>
              <a className={`${styles.btn} ${styles.btnPrimary}`} href={MANIFEST_URL} download={MANIFEST_FILENAME}>
                ⤓ Download manifest
              </a>
              <a
                className={`${styles.btn} ${styles.btnSecondary}`}
                href="#excel-setup"
              >
                Setup steps
              </a>
            </div>
          </article>
        </section>

        <section className={styles.section}>
          <h2 className={styles.h2}>How it works</h2>
          <p className={styles.p}>
            The add-in is an <strong>edge consumer</strong>, exactly like this GUI and the SDK: it
            shapes a request in the single Celnet contract, sends it over the{" "}
            <code className={styles.inlineCode}>celnet-server</code> WebSocket mirror, and renders the
            server&apos;s typed result. It adds <strong>no pricing of its own</strong> — every number
            is the server&apos;s core value, so a workbook never drifts from the platform.
          </p>
          <ul className={styles.bullets}>
            <li>
              <strong>One polymorphic surface.</strong> <code className={styles.inlineCode}>CELNET.INSTRUMENT</code>{" "}
              builds an opaque token for any product family on any asset class; the verbs{" "}
              <code className={styles.inlineCode}>PRICE</code> / <code className={styles.inlineCode}>GREEKS</code> /{" "}
              <code className={styles.inlineCode}>RFQ</code> / <code className={styles.inlineCode}>SUBSCRIBE</code> price it.
            </li>
            <li>
              <strong>Live &amp; stale-aware.</strong> Streaming cells re-tick and flip to a visible{" "}
              <em>stale</em> state on a missed heartbeat — never shown live when the feed has frozen.
            </li>
            <li>
              <strong>Convention-first.</strong> Every value spills with the resolved FX convention —
              the cure for option mismarks.
            </li>
            <li>
              <strong>Click-to-trade in the task pane.</strong> A cell never trades; the ticket pane
              binds a live RFS token for execution and the mark Stage → Confirm flow.
            </li>
          </ul>

          <table className={styles.table}>
            <thead>
              <tr>
                <th>Function</th>
                <th>What it returns</th>
              </tr>
            </thead>
            <tbody>
              {FUNCTIONS.map((r) => (
                <tr key={r.fn}>
                  <td className={styles.fn}>{r.fn}</td>
                  <td>{r.what}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <p className={styles.p}>Try these once the add-in is loaded:</p>
          <pre className={styles.code}>{EXAMPLE_FORMULAS}</pre>
        </section>

        <section className={styles.section} id="excel-setup">
          <h2 className={styles.h2}>Set up your Excel</h2>
          <ol className={styles.steps}>
            <li className={styles.step}>
              <div className={styles.stepBody}>
                <p className={styles.stepTitle}>Run the add-in</p>
                <p className={styles.stepText}>
                  The add-in is served over HTTPS. Trust a dev cert once with{" "}
                  <code className={styles.inlineCode}>npx office-addin-dev-certs install</code>, then in the{" "}
                  <code className={styles.inlineCode}>excel/</code> project run{" "}
                  <code className={styles.inlineCode}>npm install &amp;&amp; npm run dev</code> — it serves{" "}
                  <code className={styles.inlineCode}>https://localhost:3000</code>. (A deployed install
                  substitutes its own host; the manifest URLs must be reachable.)
                </p>
              </div>
            </li>
            <li className={styles.step}>
              <div className={styles.stepBody}>
                <p className={styles.stepTitle}>Point it at your edge</p>
                <p className={styles.stepText}>
                  The functions dial <code className={styles.inlineCode}>{DEFAULT_WS_ENDPOINT}</code> by
                  default. To use another edge, set{" "}
                  <code className={styles.inlineCode}>globalThis.CELNET_WS_ENDPOINT</code> at sideload
                  time (or from the task pane). Run a <code className={styles.inlineCode}>celnet-server</code>{" "}
                  and note its <code className={styles.inlineCode}>WS-mirror ws://HOST:PORT</code> line.
                </p>
              </div>
            </li>
            <li className={styles.step}>
              <div className={styles.stepBody}>
                <p className={styles.stepTitle}>Sideload the manifest</p>
                <p className={styles.stepText}>
                  <strong>Excel on the web:</strong> Insert → Add-ins → Upload My Add-in → choose the{" "}
                  downloaded <code className={styles.inlineCode}>manifest.xml</code>.{" "}
                  <strong>Mac desktop:</strong> copy it to{" "}
                  <code className={styles.inlineCode}>~/Library/Containers/com.microsoft.Excel/Data/Documents/wef/</code>{" "}
                  and restart Excel.{" "}
                  <strong>Windows desktop:</strong> add it to a Trusted Add-in Catalog
                  (File → Options → Trust Center), then Insert → My Add-ins.
                </p>
              </div>
            </li>
            <li className={styles.step}>
              <div className={styles.stepBody}>
                <p className={styles.stepTitle}>Price something</p>
                <p className={styles.stepText}>
                  In a cell type{" "}
                  <code className={styles.inlineCode}>=CELNET.PRICE(&quot;EURUSD&quot;,&quot;1Y&quot;,1.12,&quot;C&quot;,1000000)</code>{" "}
                  and press Enter. Open the <strong>Celnet Ticket</strong> task pane (Home → Celnet FX
                  Options) for RFQ, click-to-trade, and the surface mark flow. Or just open the example
                  workbook above.
                </p>
              </div>
            </li>
          </ol>

          <p className={styles.note}>
            The downloaded manifest is the self-host / development manifest (its URLs point at{" "}
            <code className={styles.inlineCode}>https://localhost:3000</code>), matching the add-in&apos;s
            dev server. A deployed add-in ships a manifest pointing at its own host. Office.js behaves
            identically across Excel on the web, Windows, and Mac.
          </p>
        </section>
      </div>
    </div>
  );
}
