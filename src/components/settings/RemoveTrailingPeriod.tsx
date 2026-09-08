import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface RemoveTrailingPeriodProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const RemoveTrailingPeriod: React.FC<RemoveTrailingPeriodProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("remove_trailing_period") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(nextEnabled) =>
          updateSetting("remove_trailing_period", nextEnabled)
        }
        isUpdating={isUpdating("remove_trailing_period")}
        label={t("settings.advanced.removeTrailingPeriod.title")}
        description={t("settings.advanced.removeTrailingPeriod.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
