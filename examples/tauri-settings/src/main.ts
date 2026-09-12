import { AppSettings } from "./bindings/amethystate";

interface ProxyProfile {
  name: string;
  address: string;
  port: number;
  enabled: boolean;
}

async function initApp() {

  const settings = await AppSettings.load();

  const usernameInput = document.querySelector("#username-input") as HTMLInputElement | null;
  const counterInput = document.querySelector("#counter-input") as HTMLInputElement | null;
  const addressDisplay = document.querySelector("#address-display");

  let currentUsername = settings.username.value ?? "";
  let currentCounter = settings.counter.value ?? 0;

  function updateAddress() {
    if (addressDisplay) {
      addressDisplay.textContent = `${currentUsername}:${currentCounter}`;
    }
  }

  if (usernameInput) {
    usernameInput.value = currentUsername;
    usernameInput.addEventListener("input", () => {
      settings.username.value = usernameInput.value;
    });
    settings.username.subscribe((val) => {
      currentUsername = val ?? "";
      if (document.activeElement !== usernameInput) {
        usernameInput.value = currentUsername;
      }
      updateAddress();
    });
  }

  if (counterInput) {
    counterInput.value = currentCounter.toString();
    counterInput.addEventListener("input", () => {
      const num = parseInt(counterInput.value, 10);
      if (!isNaN(num)) {
        settings.counter.value = num;
      }
    });
    settings.counter.subscribe((val) => {
      currentCounter = val ?? 0;
      if (document.activeElement !== counterInput) {
        counterInput.value = currentCounter.toString();
      }
      updateAddress();
    });
  }

  updateAddress();

  const themeModeSelect = document.querySelector("#theme-mode-select") as HTMLSelectElement | null;
  const themeBgInput = document.querySelector("#theme-bg-input") as HTMLInputElement | null;
  const themeFgInput = document.querySelector("#theme-fg-input") as HTMLInputElement | null;

  if (themeModeSelect) {
    themeModeSelect.value = settings.theme.mode.value ?? "light";
    themeModeSelect.addEventListener("change", () => {
      settings.theme.mode.value = themeModeSelect.value;
    });
    settings.theme.mode.subscribe((val) => {
      themeModeSelect.value = val ?? "light";
    });
  }

  if (themeBgInput) {
    themeBgInput.value = settings.theme.background.value ?? "#ffffff";
    themeBgInput.addEventListener("input", () => {
      settings.theme.background.value = themeBgInput.value;
    });
    settings.theme.background.subscribe((val) => {
      themeBgInput.value = val ?? "#ffffff";
    });
  }

  if (themeFgInput) {
    themeFgInput.value = settings.theme.foreground.value ?? "#000000";
    themeFgInput.addEventListener("input", () => {
      settings.theme.foreground.value = themeFgInput.value;
    });
    settings.theme.foreground.subscribe((val) => {
      themeFgInput.value = val ?? "#000000";
    });
  }

  const proxyNameInput = document.querySelector("#proxy-name-input") as HTMLInputElement | null;
  const proxyAddressInput = document.querySelector("#proxy-address-input") as HTMLInputElement | null;
  const proxyPortInput = document.querySelector("#proxy-port-input") as HTMLInputElement | null;
  const proxyEnabledCheckbox = document.querySelector("#proxy-enabled-checkbox") as HTMLInputElement | null;
  const proxyStatus = document.querySelector("#proxy-status");

  const defaultProxy: ProxyProfile = { name: "", address: "", port: 80, enabled: false };
  let currentProxy: ProxyProfile = settings.proxy.value ?? defaultProxy;

  function updateProxyUI(p: ProxyProfile) {
    if (proxyNameInput && document.activeElement !== proxyNameInput) {
      proxyNameInput.value = p.name ?? "";
    }
    if (proxyAddressInput && document.activeElement !== proxyAddressInput) {
      proxyAddressInput.value = p.address ?? "";
    }
    if (proxyPortInput && document.activeElement !== proxyPortInput) {
      proxyPortInput.value = (p.port ?? 80).toString();
    }
    if (proxyEnabledCheckbox) {
      proxyEnabledCheckbox.checked = !!p.enabled;
    }
    if (proxyStatus) {
      proxyStatus.textContent = `${p.address ?? ""}:${p.port ?? 80} — ${p.enabled ? "active" : "inactive"}`;
      proxyStatus.setAttribute("style", `color: ${p.enabled ? "green" : "red"}`);
    }
  }


  const setProxyField = (updater: (p: ProxyProfile) => void) => {
    const updated = { ...currentProxy };
    updater(updated);
    settings.proxy.value = updated;
  };

  if (proxyNameInput) {
    proxyNameInput.addEventListener("input", () => {
      setProxyField((p) => { p.name = proxyNameInput.value; });
    });
  }
  if (proxyAddressInput) {
    proxyAddressInput.addEventListener("input", () => {
      setProxyField((p) => { p.address = proxyAddressInput.value; });
    });
  }
  if (proxyPortInput) {
    proxyPortInput.addEventListener("input", () => {
      const val = parseInt(proxyPortInput.value, 10);
      if (!isNaN(val)) {
        setProxyField((p) => { p.port = val; });
      }
    });
  }
  if (proxyEnabledCheckbox) {
    proxyEnabledCheckbox.addEventListener("change", () => {
      setProxyField((p) => { p.enabled = proxyEnabledCheckbox.checked; });
    });
  }

  settings.proxy.subscribe((val) => {
    currentProxy = val ?? defaultProxy;
    updateProxyUI(currentProxy);
  });

  updateProxyUI(currentProxy);

  const envList = document.querySelector("#env-list");
  const envNewKey = document.querySelector("#env-new-key") as HTMLInputElement | null;
  const envNewVal = document.querySelector("#env-new-val") as HTMLInputElement | null;
  const envAddBtn = document.querySelector("#env-add-btn");

  function renderEnvMap() {
    if (!envList) return;
    envList.innerHTML = "";

    const entries = settings.env.entries;
    for (const [key, val] of entries) {
      const row = document.createElement("div");
      row.className = "env-row";

      const keyCode = document.createElement("code");
      keyCode.textContent = key;

      const span = document.createElement("span");
      span.textContent = " = ";

      const valCode = document.createElement("code");
      valCode.textContent = val;

      const deleteBtn = document.createElement("button");
      deleteBtn.textContent = "✕";
      deleteBtn.addEventListener("click", () => {
        settings.env.removeSync(key);
      });

      row.appendChild(keyCode);
      row.appendChild(span);
      row.appendChild(valCode);
      row.appendChild(deleteBtn);
      envList.appendChild(row);
    }
  }


  if (envAddBtn && envNewKey && envNewVal) {
    envAddBtn.addEventListener("click", () => {
      const key = envNewKey.value.trim();
      const val = envNewVal.value.trim();
      if (key === "") return;

      settings.env.setSync(key, val);
      envNewKey.value = "";
      envNewVal.value = "";
    });
  }


  settings.env.subscribeAny(() => {
    renderEnvMap();
  });

  renderEnvMap();
}

window.addEventListener("DOMContentLoaded", () => {
  initApp().catch(console.error);
});