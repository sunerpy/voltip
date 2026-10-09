// 记录, the second tab (apps/mobile's History): what the phone recognised itself, searched and
// filtered by the core (`history_query`), newest first and grouped by day, with the counts of today,
// this week, this month and all time. Chips at the top show a paired computer's history instead:
// the phone's copy of it (docs/dictation.md §20.8), read-only, with the copy's state and no counts.
import { type RouteProp, useRoute } from "@react-navigation/native";
import {
  HISTORY_FILTERS,
  type HistoryBucket,
  type HistoryFilter,
  durationParts,
  formatCount,
  groupByDay,
  historyFilterLabel,
  mirrorStateText,
  outcomeLabel,
  sceneLabel,
  shortClockLabel,
} from "@voltip/shared";
import { useMemo, useState } from "react";
import { ScrollView, View } from "react-native";
import { Chip, Icon, Searchbar, Text, TouchableRipple } from "react-native-paper";

import { useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useDebounced, useHistoryList } from "../hooks/useHistoryList";
import { useHomeStats } from "../hooks/useHomeStats";
import { type TabParams, useRootNavigation } from "../routes";
import { Button } from "../ui/Button";
import { CARD_RADIUS, EmptyState, Mono, Page, RowDivider, Section, useAppTheme } from "../ui/kit";

/** How long the search waits after the last keystroke, as on the desktop. */
const SEARCH_DEBOUNCE_MS = 200;

function SavedTime({ ms }: { ms: number }) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const [before = "", after = ""] = t("home.tiles.saved", { time: "\u0000" }).split("\u0000");
  return (
    <Text>
      {before.trim().length > 0 && (
        <Text
          variant="labelSmall"
          style={{ color: theme.colors.onSurfaceVariant }}>{`${before.trim()} `}</Text>
      )}
      {durationParts(ms, locale).map((part, i) => (
        <Text key={part.unit}>
          {i > 0 ? " " : ""}
          <Text variant="titleMedium">{part.value}</Text>
          <Text
            variant="labelSmall"
            style={{ color: theme.colors.onSurfaceVariant }}>{` ${part.unit}`}</Text>
        </Text>
      ))}
      {after.trim().length > 0 && (
        <Text
          variant="labelSmall"
          style={{ color: theme.colors.onSurfaceVariant }}>{` ${after.trim()}`}</Text>
      )}
    </Text>
  );
}

function Tile({ id, label, bucket }: { id: string; label: string; bucket: HistoryBucket }) {
  const theme = useAppTheme();
  const { t } = useI18n();
  return (
    <View
      testID={`phone-history-stat-${id}`}
      style={{
        width: "48.5%",
        gap: 4,
        padding: 12,
        borderRadius: CARD_RADIUS,
        backgroundColor: theme.colors.surface,
        borderWidth: 1,
        borderColor: theme.colors.outlineVariant,
      }}>
      <View style={{ flexDirection: "row", justifyContent: "space-between", gap: 8 }}>
        <Text variant="labelSmall" style={{ color: theme.voltip.subtle }}>
          {label}
        </Text>
        <Text
          variant="labelSmall"
          style={{ color: theme.colors.onSurfaceVariant }}
          numberOfLines={1}>
          {t("home.session.count", { n: formatCount(bucket.count) })}
        </Text>
      </View>
      <SavedTime ms={bucket.savedMs} />
    </View>
  );
}

export function History() {
  const theme = useAppTheme();
  const navigation = useRootNavigation();
  const route = useRoute<RouteProp<TabParams, "History">>();
  const { t, locale } = useI18n();
  const state = useUiState();
  const asked = route.params?.desktop;
  const desktop = state.mirrors.some((m) => m.desktop === asked) ? asked : undefined;
  const copy = state.mirrors.find((m) => m.desktop === desktop);
  const total = copy === undefined ? state.history_total : copy.entries;
  const retention = state.settings.history;
  // Milliseconds: `useNow` is Unix seconds, and the days, the counts and the filters take ms.
  const now = useNow() * 1000;
  const stats = useHomeStats(now);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const search = useDebounced(query, SEARCH_DEBOUNCE_MS);
  const list = useHistoryList(filter, search, now, desktop);
  const groups = useMemo(() => groupByDay(list.entries, now, locale), [list.entries, now, locale]);
  const filterLabel = (f: HistoryFilter) =>
    f === "failed" ? t("mobile.history.failed") : historyFilterLabel(f, t);
  const show = (next: string | undefined) => {
    navigation.navigate("Tabs", {
      screen: "History",
      params: next === undefined ? {} : { desktop: next },
    });
  };

  return (
    <Page testID="phone-history" onEndReached={list.more ? list.loadMore : undefined}>
      {state.mirrors.length > 0 && (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={{ gap: 8 }}
          accessibilityLabel={t("mirror.source")}>
          <Chip
            selected={desktop === undefined}
            showSelectedCheck
            onPress={() => show(undefined)}
            icon="cellphone">
            {t("mirror.thisPhone")}
          </Chip>
          {state.mirrors.map((m) => (
            <Chip
              key={m.desktop}
              selected={desktop === m.desktop}
              showSelectedCheck
              onPress={() => show(m.desktop)}
              icon="monitor">
              {m.name}
            </Chip>
          ))}
        </ScrollView>
      )}
      {copy === undefined ? (
        <View style={{ gap: 8 }}>
          <View
            style={{
              flexDirection: "row",
              flexWrap: "wrap",
              justifyContent: "space-between",
              rowGap: 8,
            }}
            testID="phone-history-stats">
            <Tile id="today" label={t("home.tiles.today")} bucket={stats.today} />
            <Tile id="week" label={t("home.tiles.week")} bucket={stats.week} />
            <Tile id="month" label={t("home.tiles.month")} bucket={stats.month} />
            <Tile id="total" label={t("home.tiles.total")} bucket={stats.total} />
          </View>
          <TouchableRipple
            testID="phone-history-retention"
            accessibilityRole="button"
            onPress={() => {
              navigation.navigate("HistorySettings");
            }}
            style={{ borderRadius: 8 }}>
            <View
              style={{
                flexDirection: "row",
                alignItems: "center",
                gap: 8,
                minHeight: 40,
                paddingHorizontal: 4,
              }}>
              <Text variant="bodySmall" style={{ flex: 1, color: theme.colors.onSurfaceVariant }}>
                {retention.enabled
                  ? t("history.banner.retention", { keep: formatCount(retention.keep) })
                  : t("history.banner.off", { n: formatCount(total) })}
              </Text>
              <Icon source="chevron-right" size={18} color={theme.voltip.subtle} />
            </View>
          </TouchableRipple>
        </View>
      ) : (
        <Text
          variant="bodySmall"
          testID="phone-history-mirror-state"
          style={{ color: theme.colors.onSurfaceVariant, paddingHorizontal: 4 }}>
          {mirrorStateText(copy, Math.floor(now / 1000), locale)}
        </Text>
      )}
      <Searchbar
        testID="phone-history-search"
        placeholder={t("mobile.history.search")}
        accessibilityLabel={t("history.search.label")}
        value={query}
        onChangeText={setQuery}
        mode="bar"
      />
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={{ gap: 8 }}
        accessibilityLabel={t("history.filter.label")}>
        {HISTORY_FILTERS.map((f) => (
          <Chip
            key={f}
            selected={filter === f}
            showSelectedCheck
            onPress={() => setFilter(f)}
            testID={`history-filter-${f}`}>
            {filterLabel(f)}
          </Chip>
        ))}
      </ScrollView>
      {total === 0 && (
        <Section>
          <EmptyState
            icon="history"
            title={copy === undefined ? t("history.empty.none") : t("mirror.empty")}>
            {copy === undefined ? t("mobile.history.emptyBody") : t("mirror.emptyBody")}
          </EmptyState>
        </Section>
      )}
      {total > 0 && list.settled && list.matching === 0 && (
        <Section>
          <EmptyState
            icon="magnify"
            title={
              search
                ? t("history.empty.noMatch", { query: search })
                : t("history.empty.noneInFilter", { filter: filterLabel(filter) })
            }>
            {search
              ? t("history.empty.noMatchBody", { n: formatCount(total) })
              : filter === "starred" && copy === undefined
                ? t("mobile.history.starHint")
                : t("history.empty.rangeHint")}
          </EmptyState>
          {search.length > 0 && (
            <Button
              style={{ marginBottom: 12 }}
              onPress={() => {
                setQuery("");
              }}>
              {t("history.empty.clearSearch")}
            </Button>
          )}
        </Section>
      )}
      {groups.map((group) => (
        <Section key={group.day} title={group.day} right={<Mono>{group.items.length}</Mono>}>
          {group.items.map((entry, i) => {
            const outcome = outcomeLabel(entry.outcome, locale);
            return (
              <View key={entry.id}>
                {i > 0 && <RowDivider />}
                <TouchableRipple
                  testID="phone-history-row"
                  accessibilityRole="button"
                  onPress={() => {
                    if (desktop === undefined) navigation.navigate("Entry", { id: entry.id });
                    else navigation.navigate("MirrorEntry", { desktop, id: entry.id });
                  }}>
                  <View style={{ paddingHorizontal: 16, paddingVertical: 12, gap: 4 }}>
                    <Text variant="bodyLarge" numberOfLines={2}>
                      {entry.text.trim().length > 0 ? entry.text : entry.raw_text}
                    </Text>
                    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                      <Mono style={{ color: theme.voltip.subtle }}>
                        {shortClockLabel(entry.at_ms)}
                      </Mono>
                      <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
                        {outcome.text}
                      </Text>
                      {entry.origin !== undefined && (
                        <Text
                          variant="bodySmall"
                          numberOfLines={1}
                          style={{ flexShrink: 1, color: theme.colors.onSurfaceVariant }}>
                          {t(`history.origin.${entry.origin.kind}`, {
                            device: entry.origin.device,
                          })}
                        </Text>
                      )}
                      {entry.scene !== undefined && (
                        <Text
                          variant="bodySmall"
                          numberOfLines={1}
                          style={{ flexShrink: 1, color: theme.colors.onSurfaceVariant }}>
                          {sceneLabel(entry.scene, locale)}
                        </Text>
                      )}
                      <View style={{ flex: 1 }} />
                      {entry.starred && (
                        <Icon source="star" size={14} color={theme.colors.primary} />
                      )}
                    </View>
                  </View>
                </TouchableRipple>
              </View>
            );
          })}
        </Section>
      ))}
      {list.more && (
        <View style={{ alignItems: "center", gap: 4 }}>
          <Button mode="outlined" onPress={list.loadMore}>
            {t("history.loadMore")}
          </Button>
          <Mono>
            {t("history.loaded", {
              shown: formatCount(list.entries.length),
              n: formatCount(list.matching),
            })}
          </Mono>
        </View>
      )}
    </Page>
  );
}
