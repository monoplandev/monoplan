import { I18nProvider as KobalteI18nProvider, useLocale } from "@kobalte/core/i18n";
import type { WorkflowState } from "./sync/store.ts";
import {
  createContext,
  createMemo,
  createSignal,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";

export type AppLanguage = "es" | "en";

const DEFAULT_LANGUAGE: AppLanguage = "en";
const COOKIE_NAME = "locale";
const MAX_AGE = 31536000;

function cookieAttrs(maxAge: number): string {
  return `path=/;max-age=${maxAge};SameSite=Lax`;
}

function readLanguage(): AppLanguage {
  try {
    const m = document.cookie.match(/(?:^|; )locale=(es|en)/);
    if (m?.[1] === "es" || m?.[1] === "en") return m[1];
  } catch {}
  return DEFAULT_LANGUAGE;
}

function writeLanguage(language: AppLanguage) {
  document.cookie = `${COOKIE_NAME}=${language};${cookieAttrs(MAX_AGE)}`;
}

const localeByLanguage: Record<AppLanguage, string> = {
  es: "es-ES",
  en: "en-US",
};

export type Messages = {
  common: {
    loading: string;
    add: string;
    close: string;
    menu: string;
    copy: string;
    /** Copy a shareable `#item_` / `#list_` URL (`spec/urls.md`). */
    copyLink: string;
    /** Context-menu move-to-list action (opens the move palette). */
    move: string;
    delete: string;
    restore: string;
    cancel: string;
    confirm: string;
    open: string;
  };
  auth: {
    signIn: string;
    signUp: string;
    email: string;
    password: string;
    derivingKeys: string;
    noAccount: string;
    haveAccount: string;
    serverMissingDeviceCredential: string;
    /** Pre-release banner shown above both sign-in and sign-up. The link
     *  text is rendered as an anchor to the mailing-list signup. */
    prereleaseNotice: string;
    prereleaseLink: string;
  };
  nav: {
    inbox: string;
    focus: string;
    /** The Upcoming view: every open item with a deadline, by day. */
    upcoming: string;
    done: string;
    bin: string;
    /** Context-menu action that archives a list (the user-facing removal
     *  from the active workspace — there is no Delete). */
    archiveList: string;
    /** Header/section indicator for an archived list. */
    archived: string;
    /** Context-menu / header action that restores an archived list. */
    unarchiveList: string;
    renameList: string;
    newList: string;
    /** Heading over the user's own workspace (Inbox + lists). Sibling
     *  headings will appear once shared workspaces exist. */
    personal: string;
    connected: string;
    disconnected: string;
    offline: string;
    synced: string;
    syncing: string;
    lastSynced: (rel: string) => string;
    seqLabel: (n: string) => string;
    itemsListsCount: (items: number, lists: number) => string;
    undo: string;
    redo: string;
    settings: string;
    hideSidebar: string;
    showSidebar: string;
    website: string;
    logOut: string;
    exportJson: string;
    exportFailed: string;
    importJson: string;
    importSucceeded: (items: number, lists: number) => string;
    importFailed: string;
  };
  workspace: {
    emptyBin: string;
    emptyBinConfirm: string;
    createWithSpace: string;
    emptyState: string;
    notes: string;
    hasNotes: string;
    markDone: string;
    markNotDone: string;
    /** Activity section under the task-dialog notes: heading over a
     *  log of plain sentences. `createdStamp` is the creation line;
     *  the completion line (shown once done) carries the elapsed span
     *  since creation, e.g. "Completed yesterday 8:06 PM after 3 hours". */
    createdStamp: (when: string) => string;
    activity: string;
    activityCompleted: (when: string, span: string) => string;
    duplicate: string;
    moveToBin: string;
    moveToList: string;
    /** Accessible name for the task dialog's lifecycle status badge —
     *  opens the workflow-state menu. */
    changeStatus: string;
    /** Placeholder / accessible name for the filter box in the move-to-list
     *  picker's popover. */
    searchLists: string;
    /** Placeholder for the standalone move-to-list palette's filter box. */
    moveItem: string;
    moveItems: string;
    /** Empty state shown when the move-to-list filter excludes every list. */
    noMatchingLists: string;
    /** Tag marking the row for the list an item is already filed under, in
     *  the move-to-list picker. */
    currentList: string;
    /** Accessible name for the Done / Focus views' display-options popover
     *  trigger. */
    displayOptions: string;
    /** Switch label (Done / Focus options popover) toggling the origin-list
     *  badge. */
    showOriginList: string;
    /** Switch label (list display options) toggling the per-row lifecycle
     *  state badge on the flat list view. */
    showState: string;
    /** Done-view header button that opens the modal to record a completed
     *  item directly (defaults to Inbox). */
    log: string;
    /** Title-field placeholder / indicator shown when the creation modal is
     *  logging an already-completed item. */
    logCompleted: string;
    /** Accessible name for the list-icon picker trigger in the header. */
    listIcon: string;
    /** Label for the button that clears a list's custom icon. */
    removeIcon: string;
  };
  /** Chrome around the emoji picker. The emoji dataset itself is
   *  English-only (see `emoji/data.ts`), so emoji names and search terms are
   *  not localised — but the surrounding UI is. */
  emoji: {
    /** Placeholder + accessible name for the picker's search field. */
    search: string;
    /** Accessible name for the category tab strip. */
    category: string;
    /** Tab label for the recently-picked row. */
    recent: string;
    /** Grid placeholder while the dataset is being fetched. */
    loading: string;
    /** Grid placeholder when the dataset fetch failed (e.g. cold offline). */
    loadFailed: string;
    /** Grid placeholder when a search matches nothing. */
    noResults: string;
    /** Emojibase category names, keyed by `EMOJI_GROUPS[].labelKey`. */
    groups: {
      smileys: string;
      people: string;
      nature: string;
      food: string;
      travel: string;
      activities: string;
      objects: string;
      symbols: string;
      flags: string;
    };
  };
  board: {
    viewAsBoard: string;
    viewAsList: string;
    /** Header labels for the five fixed board lanes (spec/board.md). */
    backlogLane: string;
    todoLane: string;
    inProgressLane: string;
    reviewLane: string;
    doneLane: string;
    /** View-mode popover section label for the open-lane visibility
     *  toggles (client-local lane hiding, spec/board.md). */
    lanes: string;
    addItem: string;
    /** Accessible name for the list/board view-mode segmented control. */
    viewMode: string;
    /** Short segment labels for that control. */
    list: string;
    board: string;
    /** Done-lane row label in the view-mode popover's lane list. */
    showDoneColumn: string;
    /** Accessible labels for a lane row's eye button (view-mode popover). */
    showLane: (lane: string) => string;
    hideLane: (lane: string) => string;
    /** Popover action saving the current view as this list's default on
     *  every device (spec/board.md). */
    saveAsDefault: string;
    /** Disabled state of that action: this view already is the default. */
    savedAsDefault: string;
  };
  deadline: {
    /** Section label / accessible name for the deadline control. */
    label: string;
    /** Task-dialog badge label when no deadline is set. */
    unset: string;
    /** Badge label when the deadline is before today. */
    overdue: string;
    /** Badge + quick-action label for today's date. */
    today: string;
    /** Badge + quick-action label for tomorrow's date. */
    tomorrow: string;
    /** Quick action that removes the deadline. */
    clear: string;
    /** Context-menu action that removes the deadline. */
    remove: string;
    /** Context-menu action that opens the calendar to pick a date. */
    setDate: string;
    /** Title of the calendar modal. */
    dialogTitle: string;
    /** Accessible label for the calendar's previous-month button. */
    prevMonth: string;
    /** Accessible label for the calendar's next-month button. */
    nextMonth: string;
  };
  when: {
    /** Section label / accessible name for the planned-date control. */
    label: string;
    /** Task-dialog date input placeholder when no planned date is set. */
    placeholder: string;
    /** Label of the task-dialog switch between an all-day and a timed `when`. */
    allDay: string;
    /** Badge + quick-action label for today's date. */
    today: string;
    /** Badge + quick-action label for tomorrow's date. */
    tomorrow: string;
    /** Quick action / context-menu action that removes the planned date. */
    remove: string;
    /** Context-menu action that opens the calendar to pick a date. */
    setDate: string;
    /** Title of the calendar modal. */
    dialogTitle: string;
    /** Label of the optional time field under the calendar. */
    time: string;
    /** Button beside the time field that blanks it (back to all-day). */
    clearTime: string;
    /** Accessible name + placeholder of the end-time field after the
     *  start time; typing an end stores a duration. */
    end: string;
    /** Button beside the end field that removes the duration. */
    clearEnd: string;
  };
  upcoming: {
    /** Upcoming view's Today group when nothing is due today. */
    emptyToday: string;
  };
  sidePanel: {
    /** Accessible name of the desktop side panel. */
    title: string;
    /** App-menu toggle labels for the desktop side panel. */
    show: string;
    hide: string;
    toPanel: string;
    toModal: string;
    /** Panel heading while more than one row is selected (`n` ≥ 2). */
    selectedCount: (n: number) => string;
    /** Accessible name of the bulk-action group under that heading. */
    selectionActions: string;
    /** Bulk-action button that drops the whole selection. */
    clearSelection: string;
  };
  shortcuts: {
    title: string;
    newItem: string;
    openItem: string;
    toggleDone: string;
    toggleFocus: string;
    /** The bare `m` move-to-list palette. */
    moveToList: string;
    duplicate: string;
    copy: string;
    undo: string;
    redo: string;
    bin: string;
    switchList: string;
    switchLane: string;
    /** The 1–4 digit jumps to the fixed nav views. */
    goToView: string;
    find: string;
    showShortcuts: string;
  };
  find: {
    placeholder: string;
    noMatches: string;
    /** Footer key hints. */
    hintSelect: string;
    hintOpen: string;
    hintMove: string;
    hintClose: string;
  };
  focus: {
    /** Add-to-focus context-menu action. */
    add: string;
    /** Remove-from-focus × affordance / context menu. */
    remove: string;
    /** Static Focus-membership badge shown on pinned list rows. */
    badge: string;
    /** Context-menu jump from the Focus lens to the item's home list. */
    showInList: (list: string) => string;
    /** Context-menu jump from a list / board to the item in the Focus lens. */
    showInFocus: string;
    /** Empty-state hint shown when the Focus lens has no visible refs. */
    empty: string;
  };
  settings: {
    general: string;
    account: string;
    devices: string;
    language: string;
    languageSpanish: string;
    languageEnglish: string;
    theme: string;
    auto: string;
    light: string;
    dark: string;
    /** List row density: taller rows with dividers vs tight rows. */
    density: string;
    densityStandard: string;
    densityCompact: string;
    showListCounts: string;
    /** 12 / 24-hour clock preference; "Auto" reuses `auto`. */
    timeFormat: string;
    timeFormat12: string;
    timeFormat24: string;
    localOnlyAccount: string;
    loginToSeeDevices: string;
    email: string;
    thisDevice: string;
    lastSeen: string;
    deviceSeq: (acked: number, head: number) => string;
    deviceActions: string;
    renameDevice: string;
    revoke: string;
    revoking: string;
    revokeDeviceConfirm: (name: string) => string;
    failedToRenameDevice: string;
    failedToRevokeDevice: string;
    failedToLoadDevices: string;
  };
  relative: {
    justNow: string;
    secondsAgo: (n: number) => string;
    minutesAgo: (n: number) => string;
    hoursAgo: (n: number) => string;
    yesterdayAt: (time: string) => string;
    daysAgo: (n: number) => string;
  };
};

const messagesByLanguage: Record<AppLanguage, Messages> = {
  es: {
    common: {
      loading: "Cargando…",
      add: "Añadir",
      close: "Cerrar",
      menu: "Menú",
      copy: "Copiar",
      copyLink: "Copiar enlace",
      move: "Mover",
      delete: "Eliminar",
      restore: "Restaurar",
      cancel: "Cancelar",
      confirm: "Confirmar",
      open: "Abrir",
    },
    auth: {
      signIn: "Iniciar sesión",
      signUp: "Crear cuenta",
      email: "Correo",
      password: "Contraseña",
      derivingKeys: "Derivando claves…",
      noAccount: "¿No tienes cuenta? Crea una",
      haveAccount: "¿Ya tienes cuenta? Inicia sesión",
      serverMissingDeviceCredential: "el servidor no devolvió una credencial de dispositivo",
      prereleaseNotice:
        "Esta es una versión preliminar. Los datos se borrarán con regularidad.",
      prereleaseLink:
        "Suscríbete a nuestra lista de correo para saber cuándo se lance Monoplan.",
    },
    nav: {
      inbox: "Bandeja de entrada",
      focus: "Enfoque",
      upcoming: "Calendario",
      done: "Hecho",
      bin: "Papelera",
      archiveList: "Archivar",
      archived: "Archivada",
      unarchiveList: "Restaurar",
      renameList: "Renombrar",
      newList: "Nueva lista",
      personal: "Personal",
      connected: "Conectado",
      disconnected: "Desconectado",
      offline: "Sin conexión",
      synced: "Sincronizado",
      syncing: "Sincronizando",
      lastSynced: (rel) => `Sincronizado ${rel}`,
      seqLabel: (n) => `seq #${n}`,
      itemsListsCount: (items, lists) =>
        `${items} elemento${items === 1 ? "" : "s"}, ${lists} lista${lists === 1 ? "" : "s"}`,
      undo: "Deshacer",
      redo: "Rehacer",
      settings: "Ajustes",
      hideSidebar: "Ocultar barra lateral",
      showSidebar: "Mostrar barra lateral",
      website: "Sitio web de Monoplan",
      logOut: "Cerrar sesión",
      exportJson: "Exportar JSON",
      exportFailed: "No se pudo exportar",
      importJson: "Importar JSON",
      importSucceeded: (items, lists) =>
        `Importado: ${items} elemento${items === 1 ? "" : "s"}, ${lists} lista${lists === 1 ? "" : "s"}`,
      importFailed: "No se pudo importar el archivo",
    },
    workspace: {
      emptyBin: "Vaciar papelera",
      emptyBinConfirm: "¿Seguro que quieres borrar permanentemente los elementos de la papelera?",
      createWithSpace: "Pulsa Espacio para crear un elemento nuevo",
      emptyState: "Nada aquí.",
      notes: "Notas",
      hasNotes: "Tiene notas",
      markDone: "Marcar como hecho",
      markNotDone: "Marcar como no hecho",
      createdStamp: (when) => `Creado ${when}`,
      activity: "Actividad",
      activityCompleted: (when, span) => `Completado ${when} tras ${span}`,
      duplicate: "Duplicar",
      moveToBin: "Mover a la papelera",
      moveToList: "Mover a la lista",
      changeStatus: "Cambiar estado",
      searchLists: "Buscar listas",
      moveItem: "Mover elemento",
      moveItems: "Mover elementos",
      noMatchingLists: "No hay listas coincidentes",
      currentList: "Actual",
      displayOptions: "Opciones de visualización",
      showOriginList: "Mostrar lista",
      showState: "Mostrar estado",
      log: "Registrar",
      logCompleted: "Registrar elemento completado",
      listIcon: "Icono de la lista",
      removeIcon: "Quitar icono",
    },
    emoji: {
      search: "Buscar emoji",
      category: "Categoría",
      recent: "Recientes",
      loading: "Cargando emoji…",
      loadFailed: "No se pudieron cargar los emoji",
      noResults: "No se encontraron emoji",
      groups: {
        smileys: "Caras y emociones",
        people: "Personas y cuerpo",
        nature: "Animales y naturaleza",
        food: "Comida y bebida",
        travel: "Viajes y lugares",
        activities: "Actividades",
        objects: "Objetos",
        symbols: "Símbolos",
        flags: "Banderas",
      },
    },
    board: {
      viewAsBoard: "Vista de tablero",
      viewAsList: "Vista de lista",
      backlogLane: "Pendiente",
      todoLane: "Preparado",
      inProgressLane: "En curso",
      reviewLane: "Revisión",
      doneLane: "Hecho",
      lanes: "Carriles",
      addItem: "Añadir elemento",
      viewMode: "Modo de vista",
      list: "Lista",
      board: "Tablero",
      showDoneColumn: "Hecho",
      showLane: (lane) => `Mostrar ${lane}`,
      hideLane: (lane) => `Ocultar ${lane}`,
      saveAsDefault: "Guardar como predeterminada",
      savedAsDefault: "Vista predeterminada",
    },
    deadline: {
      label: "Fecha límite",
      unset: "Fecha límite",
      overdue: "Vencido",
      today: "Hoy",
      tomorrow: "Mañana",
      clear: "Borrar",
      remove: "Quitar fecha límite",
      setDate: "Elegir fecha límite…",
      dialogTitle: "Establecer fecha límite",
      prevMonth: "Mes anterior",
      nextMonth: "Mes siguiente",
    },
    when: {
      label: "Cuándo",
      placeholder: "Fecha",
      allDay: "Todo el día",
      today: "Hoy",
      tomorrow: "Mañana",
      remove: "Quitar fecha",
      setDate: "Elegir fecha…",
      dialogTitle: "Establecer fecha",
      time: "Hora",
      clearTime: "Quitar hora",
      end: "Fin",
      clearEnd: "Quitar fin",
    },
    upcoming: {
      emptyToday: "Nada para hoy",
    },
    sidePanel: {
      title: "Barra de contexto",
      show: "Mostrar barra de contexto",
      hide: "Ocultar barra de contexto",
      toPanel: "Abrir en la barra de contexto",
      toModal: "Abrir como diálogo",
      selectedCount: (n) => `${n} elementos seleccionados`,
      selectionActions: "Acciones sobre la selección",
      clearSelection: "Deseleccionar",
    },
    shortcuts: {
      title: "Atajos de teclado",
      newItem: "Nuevo elemento",
      openItem: "Abrir elemento",
      toggleDone: "Marcar como hecho",
      toggleFocus: "Añadir o quitar de Enfoque",
      moveToList: "Mover a la lista",
      duplicate: "Duplicar",
      copy: "Copiar",
      undo: "Deshacer",
      redo: "Rehacer",
      bin: "Mover a la papelera",
      switchList: "Cambiar de vista",
      switchLane: "Cambiar de carril",
      goToView: "Ir a Enfoque / Próximo / Hecho / Entrada",
      find: "Buscar",
      showShortcuts: "Mostrar atajos",
    },
    find: {
      placeholder: "Buscar",
      noMatches: "Sin resultados",
      hintSelect: "Navegar",
      hintOpen: "Abrir",
      hintMove: "Mover",
      hintClose: "Cerrar",
    },
    focus: {
      add: "Enfoque",
      remove: "Quitar de Enfoque",
      badge: "Enfoque",
      showInList: (list: string) => `Ver en ${list}`,
      showInFocus: "Ver en Enfoque",
      empty:
        "Enfoque está vacío. Añade un elemento nuevo aquí, o haz clic derecho en uno existente y elige «Añadir a Enfoque», para organizar en qué estás trabajando.",
    },
    settings: {
      general: "General",
      account: "Cuenta",
      devices: "Dispositivos",
      language: "Idioma",
      languageSpanish: "Español",
      languageEnglish: "English",
      theme: "Tema",
      auto: "Auto",
      light: "Claro",
      dark: "Oscuro",
      density: "Densidad",
      densityStandard: "Estándar",
      densityCompact: "Compacta",
      showListCounts: "Mostrar contadores de listas",
      timeFormat: "Formato de hora",
      timeFormat12: "12 h",
      timeFormat24: "24 h",
      localOnlyAccount:
        "Estás usando una cuenta solo local. Usa Iniciar sesión o Crear cuenta desde el menú de la cuenta para hacer copia de seguridad de tus datos y sincronizar entre dispositivos.",
      loginToSeeDevices: "Inicia sesión para ver los dispositivos vinculados a tu cuenta.",
      email: "Correo",
      thisDevice: "Este dispositivo",
      lastSeen: "Última vez visto",
      deviceSeq: (acked, head) => `sincronizado hasta op ${acked} de ${head}`,
      deviceActions: "Acciones del dispositivo",
      renameDevice: "Renombrar",
      revoke: "Revocar",
      revoking: "Revocando…",
      revokeDeviceConfirm: (name) =>
        `¿Revocar «${name}»? Tendrá que volver a iniciar sesión para sincronizar.`,
      failedToRenameDevice: "No se pudo renombrar el dispositivo",
      failedToRevokeDevice: "No se pudo revocar el dispositivo",
      failedToLoadDevices: "No se pudieron cargar los dispositivos",
    },
    relative: {
      justNow: "ahora mismo",
      secondsAgo: (n) => `hace ${n} s`,
      minutesAgo: (n) => `hace ${n} min`,
      hoursAgo: (n) => `hace ${n} h`,
      yesterdayAt: (time) => `Ayer ${time}`,
      daysAgo: (n) => `hace ${n} día${n === 1 ? "" : "s"}`,
    },
  },
  en: {
    common: {
      loading: "Loading…",
      add: "Add",
      close: "Close",
      menu: "Menu",
      copy: "Copy",
      copyLink: "Copy link",
      move: "Move",
      delete: "Delete",
      restore: "Restore",
      cancel: "Cancel",
      confirm: "Confirm",
      open: "Open",
    },
    auth: {
      signIn: "Sign in",
      signUp: "Sign up",
      email: "Email",
      password: "Password",
      derivingKeys: "Deriving keys…",
      noAccount: "Don't have an account? Sign up",
      haveAccount: "Have an account? Sign in",
      serverMissingDeviceCredential: "server did not return a device credential",
      prereleaseNotice:
        "This is a pre-release build. Data will be regularly deleted.",
      prereleaseLink:
        "Sign up to our mailing list to get notified when Monoplan releases.",
    },
    nav: {
      inbox: "Inbox",
      focus: "Focus",
      upcoming: "Calendar",
      done: "Done",
      bin: "Bin",
      archiveList: "Archive",
      archived: "Archived",
      unarchiveList: "Unarchive",
      renameList: "Rename",
      newList: "New list",
      personal: "Personal",
      connected: "Connected",
      disconnected: "Disconnected",
      offline: "Offline",
      synced: "Synced",
      syncing: "Syncing",
      lastSynced: (rel) => `Synced ${rel}`,
      seqLabel: (n) => `seq #${n}`,
      itemsListsCount: (items, lists) =>
        `${items} item${items === 1 ? "" : "s"}, ${lists} list${lists === 1 ? "" : "s"}`,
      undo: "Undo",
      redo: "Redo",
      settings: "Settings",
      hideSidebar: "Hide sidebar",
      showSidebar: "Show sidebar",
      website: "Monoplan website",
      logOut: "Log out",
      exportJson: "Export JSON",
      exportFailed: "Could not export",
      importJson: "Import JSON",
      importSucceeded: (items, lists) =>
        `Imported ${items} item${items === 1 ? "" : "s"}, ${lists} list${lists === 1 ? "" : "s"}`,
      importFailed: "Could not import file",
    },
    workspace: {
      emptyBin: "Empty",
      emptyBinConfirm: "Are you sure you want to permanently erase items in the bin?",
      createWithSpace: "Press Space to create a new item",
      emptyState: "Nothing here.",
      notes: "Notes",
      hasNotes: "Has notes",
      markDone: "Mark as done",
      markNotDone: "Mark as not done",
      createdStamp: (when) => `Created ${when}`,
      activity: "Activity",
      activityCompleted: (when, span) => `Completed ${when} after ${span}`,
      duplicate: "Duplicate",
      moveToBin: "Move to bin",
      moveToList: "Move to list",
      changeStatus: "Change status",
      searchLists: "Search lists",
      moveItem: "Move item",
      moveItems: "Move items",
      noMatchingLists: "No matching lists",
      currentList: "Current",
      displayOptions: "Display options",
      showOriginList: "Show list",
      showState: "Show state",
      log: "Log",
      logCompleted: "Log completed item",
      listIcon: "List icon",
      removeIcon: "Remove icon",
    },
    emoji: {
      search: "Search emoji",
      category: "Category",
      recent: "Recently used",
      loading: "Loading emoji…",
      loadFailed: "Couldn't load emoji",
      noResults: "No emoji found",
      groups: {
        smileys: "Smileys & emotion",
        people: "People & body",
        nature: "Animals & nature",
        food: "Food & drink",
        travel: "Travel & places",
        activities: "Activities",
        objects: "Objects",
        symbols: "Symbols",
        flags: "Flags",
      },
    },
    board: {
      viewAsBoard: "Board view",
      viewAsList: "List view",
      backlogLane: "Backlog",
      todoLane: "Ready",
      inProgressLane: "In progress",
      reviewLane: "Review",
      doneLane: "Done",
      lanes: "Lanes",
      addItem: "Add item",
      viewMode: "View mode",
      list: "List",
      board: "Board",
      showDoneColumn: "Done",
      showLane: (lane) => `Show ${lane}`,
      hideLane: (lane) => `Hide ${lane}`,
      saveAsDefault: "Save as default",
      savedAsDefault: "Default view",
    },
    deadline: {
      label: "Deadline",
      unset: "Deadline",
      overdue: "Overdue",
      today: "Today",
      tomorrow: "Tomorrow",
      clear: "Clear",
      remove: "Remove deadline",
      setDate: "Set deadline…",
      dialogTitle: "Set deadline",
      prevMonth: "Previous month",
      nextMonth: "Next month",
    },
    when: {
      label: "When",
      placeholder: "Date",
      allDay: "All day",
      today: "Today",
      tomorrow: "Tomorrow",
      remove: "Remove date",
      setDate: "Set date…",
      dialogTitle: "Set date",
      time: "Time",
      clearTime: "Clear time",
      end: "End",
      clearEnd: "Clear end",
    },
    upcoming: {
      emptyToday: "Nothing due today",
    },
    sidePanel: {
      title: "Context sidebar",
      show: "Show context sidebar",
      hide: "Hide context sidebar",
      toPanel: "Open in context sidebar",
      toModal: "Open as dialog",
      selectedCount: (n) => `${n} items selected`,
      selectionActions: "Selection actions",
      clearSelection: "Clear selection",
    },
    shortcuts: {
      title: "Keyboard shortcuts",
      newItem: "New item",
      openItem: "Open item",
      toggleDone: "Toggle done",
      toggleFocus: "Toggle focus",
      moveToList: "Move to list",
      duplicate: "Duplicate",
      copy: "Copy",
      undo: "Undo",
      redo: "Redo",
      bin: "Move to bin",
      switchList: "Switch view",
      switchLane: "Switch lane",
      goToView: "Go to Focus / Upcoming / Done / Inbox",
      find: "Find",
      showShortcuts: "Show shortcuts",
    },
    find: {
      placeholder: "Find",
      noMatches: "No matches",
      hintSelect: "Navigate",
      hintOpen: "Open",
      hintMove: "Move",
      hintClose: "Close",
    },
    focus: {
      add: "Focus",
      remove: "Remove from Focus",
      badge: "Focus",
      showInList: (list: string) => `Show in ${list}`,
      showInFocus: "Show in Focus",
      empty: "Nothing in Focus yet.",
    },
    settings: {
      general: "General",
      account: "Account",
      devices: "Devices",
      language: "Language",
      languageSpanish: "Español",
      languageEnglish: "English",
      theme: "Theme",
      auto: "Auto",
      light: "Light",
      dark: "Dark",
      density: "Density",
      densityStandard: "Standard",
      densityCompact: "Compact",
      showListCounts: "Show list counts",
      timeFormat: "Time format",
      timeFormat12: "12h",
      timeFormat24: "24h",
      localOnlyAccount:
        "You're using a local-only account. Use Sign in or Sign up from the account menu to back up your data and sync across devices.",
      loginToSeeDevices: "Log in to see devices linked to your account.",
      email: "Email",
      thisDevice: "This device",
      lastSeen: "Last seen",
      deviceSeq: (acked, head) => `synced to op ${acked} of ${head}`,
      deviceActions: "Device actions",
      renameDevice: "Rename",
      revoke: "Revoke",
      revoking: "Revoking…",
      revokeDeviceConfirm: (name) =>
        `Revoke “${name}”? It will need to sign in again to sync.`,
      failedToRenameDevice: "Failed to rename device",
      failedToRevokeDevice: "Failed to revoke device",
      failedToLoadDevices: "Failed to load devices",
    },
    relative: {
      justNow: "just now",
      secondsAgo: (n) => `${n}s ago`,
      minutesAgo: (n) => `${n}m ago`,
      hoursAgo: (n) => `${n}h ago`,
      yesterdayAt: (time) => `Yesterday ${time}`,
      daysAgo: (n) => `${n} day${n === 1 ? "" : "s"} ago`,
    },
  },
};

type AppI18nContextValue = {
  language: Accessor<AppLanguage>;
  setLanguage: (language: AppLanguage) => void;
  localeCode: Accessor<string>;
  messages: Accessor<Messages>;
};

const AppI18nContext = createContext<AppI18nContextValue>();

export function AppI18nProvider(props: { children: JSX.Element }) {
  const [language, setLanguageSignal] = createSignal<AppLanguage>(readLanguage());
  const localeCode = createMemo(() => localeByLanguage[language()]);
  const messages = createMemo(() => messagesByLanguage[language()]);

  const setLanguage = (language: AppLanguage) => {
    setLanguageSignal(language);
    writeLanguage(language);
  };

  return (
    <AppI18nContext.Provider
      value={{
        language,
        setLanguage,
        localeCode,
        messages,
      }}
    >
      <KobalteI18nProvider locale={localeCode()}>
        {props.children}
      </KobalteI18nProvider>
    </AppI18nContext.Provider>
  );
}

export function useAppI18n(): {
  m: Accessor<Messages>;
  language: Accessor<AppLanguage>;
  setLanguage: (language: AppLanguage) => void;
  locale: Accessor<string>;
  direction: Accessor<"ltr" | "rtl">;
} {
  const ctx = useContext(AppI18nContext);
  if (!ctx) throw new Error("missing AppI18nProvider");
  const { locale, direction } = useLocale();
  return {
    m: ctx.messages,
    language: ctx.language,
    setLanguage: ctx.setLanguage,
    locale,
    direction,
  };
}

/** Localized label for a workflow state: the board lane headers, the
 *  task dialog's status picker, and the list view's state badge all
 *  share it so a state reads the same everywhere. */
export function laneLabel(m: Messages, lane: WorkflowState): string {
  switch (lane) {
    case "backlog":
      return m.board.backlogLane;
    case "todo":
      return m.board.todoLane;
    case "in_progress":
      return m.board.inProgressLane;
    case "review":
      return m.board.reviewLane;
    case "done":
      return m.board.doneLane;
  }
}
