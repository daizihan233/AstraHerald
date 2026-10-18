import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { darkTheme } from "naive-ui";

// naive-ui 不自己读 prefers-color-scheme，桌面端跟随系统深浅色得自己接一下。
// 配置页和提醒窗口是两个入口，主题逻辑放这里共用。
export function useSystemTheme() {
  const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
  const isDark = ref(darkQuery.matches);
  const onSchemeChange = (event) => {
    isDark.value = event.matches;
  };
  onMounted(() => darkQuery.addEventListener("change", onSchemeChange));
  onBeforeUnmount(() => darkQuery.removeEventListener("change", onSchemeChange));

  const theme = computed(() => (isDark.value ? darkTheme : null));
  return { isDark, theme };
}
