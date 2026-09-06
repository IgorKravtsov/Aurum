import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { fetchSettings, updateSettings } from "@/api/settings";
import { setCurrency } from "@/lib/i18n";
import type { AppSettings } from "@/types";

export function useAppSettings() {
  return useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
}

export function useUpdateAppSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: Partial<AppSettings>) => updateSettings(input),
    onSuccess: (data) => {
      queryClient.setQueryData(["settings"], data);
      setCurrency(data.currency);
      // Every financial aggregate is returned in the configured display
      // currency, so changing it invalidates more than the formatter label.
      queryClient.invalidateQueries({ predicate: (query) => query.queryKey[0] !== "settings" });
    },
  });
}
