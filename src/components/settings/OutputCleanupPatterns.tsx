import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { SettingContainer } from "../ui/SettingContainer";
import { Textarea } from "../ui/Textarea";

interface OutputCleanupPatternsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const OutputCleanupPatterns: React.FC<OutputCleanupPatternsProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const patterns = getSetting("output_cleanup_patterns") ?? [];
    const [draft, setDraft] = useState(patterns.join("\n"));

    useEffect(() => {
      setDraft(patterns.join("\n"));
    }, [patterns]);

    const save = async () => {
      const nextPatterns = draft
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line.length > 0);

      if (nextPatterns.join("\n") !== patterns.join("\n")) {
        await updateSetting("output_cleanup_patterns", nextPatterns);
      }
    };

    return (
      <SettingContainer
        title={t("settings.advanced.outputCleanupPatterns.title")}
        description={t("settings.advanced.outputCleanupPatterns.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        layout="stacked"
      >
        <Textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => void save()}
          disabled={isUpdating("output_cleanup_patterns")}
          placeholder={t("settings.advanced.outputCleanupPatterns.placeholder")}
          variant="compact"
          className="w-full font-normal"
        />
      </SettingContainer>
    );
  });
