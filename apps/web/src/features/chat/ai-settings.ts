import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import i18n from "#/lib/i18n";
import { assistClient, chatKeys } from "./api";
import { errorMessage } from "./errors";

/**
 * The AI assistant's settings: the caller's (provider, own keys, consent)
 * and the operator's (staff console). Every change answers with the new
 * settings, which replace the cached ones; the capabilities follow, since
 * `assistEnabled` depends on them.
 */

export const assistAdminClient = () => api().service(chat.AssistAdminService);

export const aiKeys = {
  settings: ["chat", "assist-settings"] as const,
  instance: ["chat", "assist-instance"] as const,
};

/** The providers in the order the server lists them. */
export const AI_PROVIDERS = [
  chat.AiProvider.ANTHROPIC,
  chat.AiProvider.OPENAI,
  chat.AiProvider.OPENAI_COMPATIBLE,
] as const;

/** "Anthropic (Claude)", "OpenAI"… */
export function providerName(provider: chat.AiProvider): string {
  return i18n.t(`chat:aiAdmin.providers.${chat.AiProvider[provider]}`);
}

export function useAssistSettings() {
  return useQuery({
    queryKey: aiKeys.settings,
    queryFn: async () => (await assistClient().getAssistSettings({})).settings,
  });
}

/** A mutation of the caller's settings that stores what the server answers. */
export function useAssistSettingsChange<T>(
  change: (input: T) => Promise<{ settings?: chat.AssistSettings }>,
  done?: string,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: change,
    onSuccess: (response) => {
      queryClient.setQueryData(aiKeys.settings, response.settings);
      void queryClient.invalidateQueries({ queryKey: chatKeys.capabilities });
      if (done) toast.success(done);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
}

export function useAssistInstance(enabled: boolean) {
  return useQuery({
    queryKey: aiKeys.instance,
    queryFn: async () => (await assistAdminClient().getAssistInstance({})).instance,
    enabled,
  });
}

/** A mutation of the operator settings that stores what the server answers. */
export function useAssistInstanceChange<T>(
  change: (input: T) => Promise<{ instance?: chat.AssistInstance }>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: change,
    onSuccess: (response) => {
      queryClient.setQueryData(aiKeys.instance, response.instance);
      void queryClient.invalidateQueries({ queryKey: aiKeys.settings });
      void queryClient.invalidateQueries({ queryKey: chatKeys.capabilities });
      toast.success(i18n.t("chat:aiAdmin.saved"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
}
