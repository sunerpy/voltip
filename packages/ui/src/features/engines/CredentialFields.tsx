import {
  type CredentialDraft,
  type ProviderStatus,
  type ServiceStatus,
  credentialFieldLabel,
  credentialHelp,
  credentialSetLabel,
} from "@voltip/shared";
import { useState } from "react";
import { IconButton } from "../../components/IconButton";
import { Input } from "../../components/Input";
import { Segmented } from "../../components/Segmented";
import { useI18n } from "../../i18n/I18nProvider";

export interface CredentialFieldsProps {
  provider: ProviderStatus;
  service: ServiceStatus;
  draft: CredentialDraft;
  onChange: (draft: CredentialDraft) => void;
  /** The setup guide's smaller fields. */
  size?: "sm";
}

/** A credential of several fields (docs/dictation.md §3.15): the vendor's sets as a segmented
 *  choice when it has more than one (豆包语音's new and old console), then the chosen set's fields,
 *  one per row, the secret ones hidden until shown. A field of the stored set left blank keeps its
 *  stored value, as its placeholder says; switching sets starts the new one empty. The provider
 *  drawer and the setup guide both use it; `credentialValue` turns the draft into what is saved. */
export function CredentialFields({
  provider,
  service,
  draft,
  onChange,
  size,
}: CredentialFieldsProps) {
  const { t } = useI18n();
  const [shown, setShown] = useState<readonly string[]>([]);
  const sets = provider.credential ?? [];
  const set = sets.find((s) => s.id === draft.set) ?? sets[0];
  if (set === undefined) return null;
  const kept =
    service.key.set && service.key.source === "user" && service.key.credential_set === set.id;
  const help = credentialHelp(provider.id, t);
  return (
    <div className="flex flex-col gap-3" data-testid="credential-fields" data-set={set.id}>
      {sets.length > 1 && (
        <div className="flex flex-col gap-1">
          <span className="text-[12px] text-fg-muted">{t("engines.credential.setLabel")}</span>
          <Segmented
            size="sm"
            label={t("engines.credential.setLabel")}
            value={set.id}
            onChange={(id) => {
              if (id !== set.id) onChange({ set: id, fields: {} });
            }}
            options={sets.map((s) => ({ value: s.id, label: credentialSetLabel(s.id, t) }))}
            className="self-start"
          />
        </div>
      )}
      {set.fields.map((field) => {
        const visible = !field.secret || shown.includes(field.id);
        return (
          <Input
            key={field.id}
            label={credentialFieldLabel(field.id, t)}
            mono
            {...(size === undefined ? {} : { size })}
            type={visible ? "text" : "password"}
            autoComplete="off"
            spellCheck={false}
            value={draft.fields[field.id] ?? ""}
            placeholder={kept ? t("engines.credential.saved") : ""}
            data-testid={`credential-${field.id}`}
            onChange={(e) => {
              onChange({ set: set.id, fields: { ...draft.fields, [field.id]: e.target.value } });
            }}
            trailing={
              field.secret ? (
                <IconButton
                  icon="eye"
                  label={visible ? t("engines.credential.hide") : t("engines.credential.show")}
                  onClick={() => {
                    setShown(
                      visible ? shown.filter((id) => id !== field.id) : [...shown, field.id],
                    );
                  }}
                />
              ) : undefined
            }
          />
        );
      })}
      {help !== undefined && (
        <p className="text-[12px] leading-5 text-fg-muted" data-testid="credential-help">
          {help}
        </p>
      )}
    </div>
  );
}
