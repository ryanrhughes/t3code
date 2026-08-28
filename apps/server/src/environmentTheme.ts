/**
 * EnvironmentTheme - palettes this machine publishes for clients to follow.
 *
 * A desktop that retints its apps when the user switches system theme writes
 * `<stateDir>/themes/<id>.json`; this service watches that directory and
 * streams the published set to connected clients so a theme change lands
 * without a restart. The filename is the theme id: it stays stable while the
 * machine rewrites the colors underneath, so `defaultTheme` and a client\'s
 * selection keep pointing at the same theme across recolors. Theming is
 * cosmetic, so every failure here degrades to "not published" rather than
 * propagating.
 *
 * @module EnvironmentTheme
 */
import { EnvironmentTheme, EnvironmentThemeFile, EnvironmentThemeId } from "@t3tools/contracts";
import * as Cause from "effect/Cause";
import * as Context from "effect/Context";
import * as Duration from "effect/Duration";
import * as Effect from "effect/Effect";
import * as Exit from "effect/Exit";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as PubSub from "effect/PubSub";
import * as Ref from "effect/Ref";
import * as Schema from "effect/Schema";
import * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";

import * as ServerConfig from "./config.ts";

const decodeEnvironmentThemeFileJsonExit = Schema.decodeUnknownExit(
  Schema.fromJsonString(EnvironmentThemeFile),
);
const isEnvironmentThemeId = Schema.is(EnvironmentThemeId);

const THEME_FILE_SUFFIX = ".json";

export class EnvironmentThemeService extends Context.Service<
  EnvironmentThemeService,
  {
    /**
     * The set published right now, read from disk rather than from the
     * watcher\'s last observation: a client connecting must see what the
     * machine actually publishes even if it missed a filesystem event.
     */
    readonly current: Effect.Effect<ReadonlyArray<EnvironmentTheme>>;

    /** Sets published from here on, with repeats of the same set dropped. */
    readonly streamChanges: Stream.Stream<ReadonlyArray<EnvironmentTheme>>;
  }
>()("t3/environmentTheme/EnvironmentThemeService") {}

function sameTheme(left: EnvironmentTheme, right: EnvironmentTheme): boolean {
  if (
    left.id !== right.id ||
    left.name !== right.name ||
    left.appearance !== right.appearance ||
    left.canvas !== right.canvas ||
    left.accent !== right.accent
  ) {
    return false;
  }
  const leftColors = Object.entries(left.colors ?? {});
  const rightColors = right.colors ?? {};
  return (
    leftColors.length === Object.keys(rightColors).length &&
    leftColors.every(([role, value]) => rightColors[role] === value)
  );
}

function sameThemes(
  left: ReadonlyArray<EnvironmentTheme>,
  right: ReadonlyArray<EnvironmentTheme>,
): boolean {
  return (
    left.length === right.length && left.every((theme, index) => sameTheme(theme, right[index]!))
  );
}

const make = Effect.gen(function* () {
  const { environmentThemesDir } = yield* ServerConfig.ServerConfig;
  const fs = yield* FileSystem.FileSystem;
  const changes = yield* PubSub.unbounded<ReadonlyArray<EnvironmentTheme>>();
  const lastPublished = yield* Ref.make<ReadonlyArray<EnvironmentTheme>>([]);
  const watcherScope = yield* Scope.make("sequential");
  yield* Effect.addFinalizer(() => Scope.close(watcherScope, Exit.void));

  /** A file that is missing, unreadable, malformed, invalid, or misnamed is
   * simply not published; the rest of the set is unaffected. */
  const readThemes = Effect.gen(function* () {
    const entries = yield* fs
      .readDirectory(environmentThemesDir)
      .pipe(Effect.orElseSucceed((): Array<string> => []));

    const themes: Array<EnvironmentTheme> = [];
    for (const entry of entries.toSorted()) {
      if (!entry.endsWith(THEME_FILE_SUFFIX)) continue;
      const id = entry.slice(0, -THEME_FILE_SUFFIX.length);
      if (!isEnvironmentThemeId(id)) continue;

      const filePath = `${environmentThemesDir}/${entry}`;
      const raw = yield* fs.readFileString(filePath).pipe(Effect.orElseSucceed(() => ""));
      if (raw.trim().length === 0) continue;

      const decoded = decodeEnvironmentThemeFileJsonExit(raw);
      if (decoded._tag === "Failure") {
        yield* Effect.logWarning("ignoring invalid environment theme", {
          path: filePath,
          detail: Cause.pretty(decoded.cause),
        });
        continue;
      }
      const file = decoded.value;
      // A file with neither seeds nor colors would render as the stock
      // palette wearing a name, which reads as a bug, not a theme.
      const hasSeeds = file.canvas !== undefined && file.accent !== undefined;
      const hasColors = file.colors !== undefined && Object.keys(file.colors).length > 0;
      if (!hasSeeds && !hasColors) {
        yield* Effect.logWarning("ignoring environment theme without colors", { path: filePath });
        continue;
      }
      themes.push({ id, ...file });
    }
    return themes;
  });

  const publishIfChanged = Effect.gen(function* () {
    const themes = yield* readThemes;
    const changed = yield* Ref.modify(lastPublished, (previous) =>
      sameThemes(previous, themes) ? [false, previous] : [true, themes],
    );
    if (changed) yield* PubSub.publish(changes, themes).pipe(Effect.asVoid);
  });

  // The directory is created up front so the watcher has something to attach
  // to before the first publisher writes into it.
  yield* fs
    .makeDirectory(environmentThemesDir, { recursive: true })
    .pipe(Effect.ignoreCause({ log: true }));

  // Debounced for the same reason settings watching is: a theme script emits
  // several events per save and `fs.watch` can fire before the content is
  // flushed. Every event triggers a full re-read, so no event needs filtering.
  const watchEvents = fs.watch(environmentThemesDir).pipe(Stream.debounce(Duration.millis(100)));

  // Seeds the dedupe so a watch event that reports no actual change (a touch,
  // a rewrite with identical contents) does not retint every client.
  yield* Ref.set(lastPublished, yield* readThemes);
  yield* Stream.runForEach(watchEvents, () =>
    publishIfChanged.pipe(Effect.ignoreCause({ log: true })),
  ).pipe(Effect.ignoreCause({ log: true }), Effect.forkIn(watcherScope), Effect.asVoid);

  return {
    current: readThemes,
    get streamChanges() {
      return Stream.fromPubSub(changes);
    },
  } satisfies EnvironmentThemeService["Service"];
});

export const layer = Layer.effect(EnvironmentThemeService, make);
