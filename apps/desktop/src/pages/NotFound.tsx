import { Button, EmptyState, useT } from "@voltip/ui";
import { useRouter } from "../app/router";

export function NotFound({ path }: { path: string }) {
  const { navigate } = useRouter();
  const t = useT();
  return (
    <div className="mx-auto w-full max-w-[1440px] p-6" data-testid="page-notfound">
      <EmptyState
        title={t("notFound.title")}
        mono={path}
        icon="alert"
        actions={
          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              navigate({ name: "home" });
            }}>
            {t("notFound.home")}
          </Button>
        }>
        {t("notFound.body")}
      </EmptyState>
    </div>
  );
}
