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
    /** Recover mode: redeem the 12-word code for a new password. */
    resetPassword: string;
    newPassword: string;
    recoveryCode: string;
    forgotPassword: string;
    backToSignIn: string;
    recoveryCodeInvalid: string;
    recoveryNotEnrolled: string;
    /** Signup: tells the user a recovery code is coming and why. */
    signupRecoveryNote: string;
    /** The show-once pane after a signup that enrolled a code. */
    recoveryTitle: string;
    recoveryIntro: string;
    recoverySaved: string;
    copied: string;
    continue: string;
    /** Pre-release banner shown above both sign-in and sign-up. The link
     *  text is rendered as an anchor to the mailing-list signup. */
    prereleaseNotice: string;
    prereleaseLink: string;
  };
  nav: {
    inbox: string;
    /** The reserved `events` list (`spec/events-plan.md`). Never a nav
     *  entry: it labels events as a move destination and an origin list. */
    events: string;
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
    /** Heading over the account's Home doc (Inbox + lists). Every other
     *  open doc gets a sibling heading carrying its own name. */
    personal: string;
    /** Heading for a doc that has no name yet. */
    untitledDoc: string;
    /** The app menu's create-doc item and the dialog it opens. */
    newDoc: string;
    /** The dialog's name field placeholder. */
    newDocName: string;
    /** The dialog's submit button. */
    createDoc: string;
    /** Context-menu rename on a non-Home doc heading. */
    renameDoc: string;
    /** Label of the "..." button on a workspace heading. */
    docMenu: string;
    /** "..." menu delete on a non-Home doc heading, its confirmation
     *  (named for the doc) and the failure alert. */
    deleteDoc: string;
    deleteDocConfirm: (name: string) => string;
    /** Prompt above the type-the-name field in the delete confirmation. */
    deleteDocTypeName: (name: string) => string;
    deleteDocFailed: string;
    createDocFailed: string;
    /** Non-Home workspace menu item opening `SharingDialog`. */
    sharing: string;
    connected: string;
    disconnected: string;
    offline: string;
    synced: string;
    syncing: string;
    lastSynced: (rel: string) => string;
    seqLabel: (n: string) => string;
    epochLabel: (n: string) => string;
    itemsListsCount: (items: number, lists: number) => string;
    /** Footer popover: how many docs this session has open. */
    docsCount: (n: number) => string;
    undo: string;
    redo: string;
    settings: string;
    website: string;
    logOut: string;
    /** Sign-out confirmation (title + body); mirrors the macOS app's alert. */
    logOutTitle: string;
    logOutConfirm: string;
    exportJson: string;
    exportFailed: string;
    importJson: string;
    importSucceeded: (items: number, lists: number) => string;
    importFailed: string;
  };
  workspace: {
    /** Accessible name of the task dialog's "+" chip: the menu of
     *  registers (date, deadline, place) the item does not carry yet. */
    emptyBin: string;
    /** Title of the empty-bin confirmation dialog. */
    emptyBinTitle: string;
    emptyBinConfirm: string;
    createWithSpace: string;
    emptyState: string;
    notes: string;
    hasNotes: string;
    markDone: string;
    markNotDone: string;
    /** Cancel: the closed-but-not-completed sibling of Done
     *  (`spec/data-model.md` "Lifecycle"). `reopen` is its un-done. */
    markCancelled: string;
    reopen: string;
    /** Activity section under the task-dialog notes: a log of plain
     *  sentences (`activity` is its accessible name, not shown).
     *  `createdStamp` is the creation line; once done, the completion
     *  line ("Completed yesterday 8:06 PM") is followed by the elapsed
     *  span since creation on its own line ("3 hours elapsed"). */
    createdStamp: (when: string) => string;
    /** Joins the creation stamp to the creator's avatar: "Created 5:49 PM by ●". */
    createdBy: string;
    activity: string;
    activityCompleted: (when: string) => string;
    activityElapsed: (span: string) => string;
    /** Activity line for a cancelled item: when it was cancelled. */
    activityCancelled: (when: string) => string;
    duplicate: string;
    moveToBin: string;
    moveToList: string;
    /** Accessible name for the task dialog's lifecycle status badge —
     *  opens the workflow-state menu. */
    changeStatus: string;
    /** Row context-menu submenu that moves the target set to a workflow
     *  state (the menu twin of the task dialog's status badge). */
    status: string;
    /** Placeholder / accessible name for the filter box in the move-to-list
     *  picker's popover, naming the workspace whose lists it filters. */
    searchLists: (workspace: string) => string;
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
    /** Done view display option: span every open doc, not Home alone. */
    allDocs: string;
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
    /** Header labels for the five fixed board lanes (spec/board.md),
     *  plus the Cancelled state label (not a lane: cancelled cards sit in
     *  the Done lane, but the status picker and badges still name it). */
    backlogLane: string;
    todoLane: string;
    inProgressLane: string;
    reviewLane: string;
    doneLane: string;
    cancelledLane: string;
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
    /** Task-dialog "+" menu row that reveals the deadline band. */
    add: string;
    /** Task-dialog deadline input placeholder when no deadline is set. */
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
    /** Context-menu action that opens the item onto its date popover. */
    setDate: string;
    /** Label of the time field in the task surface's dates band. */
    time: string;
    /** ✕ beside the time field and the Remove row in its panel: blanks
     *  the time (back to all-day). */
    clearTime: string;
    /** Accessible name + placeholder of the end-time field after the
     *  start time; typing an end stores a duration. */
    end: string;
    /** Accessible name of the end-date field under the end time: the day
     *  the span ends on, picked from a calendar. */
    endDate: string;
  };
  place: {
    /** Accessible name of the task-dialog place input. */
    label: string;
    /** Placeholder of the place input when no place is set. */
    placeholder: string;
    /** ✕ beside the input, and the Remove row under the results (the
     *  only route on the mobile page): removes the place. */
    remove: string;
    /** The blank run of the field beside the label link: starts an edit. */
    edit: string;
    /** Popover row offering the typed text as the label, as is. */
    asTyped: string;
    /** Popover note while a lookup runs. */
    searching: string;
    /** Popover note when a lookup found nothing. */
    noResults: string;
    /** Popover note when a lookup failed. */
    error: string;
    /** Title of the glyph on a result that is already on one of the
     *  user's items. */
    known: string;
    /** Link under the input to the place on the map. */
    openMap: string;
    /** Attribution line at the foot of the results popover. */
    credit: string;
  };
  upcoming: {
    /** Upcoming view's Today group when nothing is due today. */
    emptyToday: string;
    /** Heading over the calendar's per-doc show / hide toggles. */
    docs: string;
    /** Header of the group holding events with no date. */
    unscheduled: string;
    /** What an Open event's status reads: the one label its four open
     *  states share in the status controls (`statusLabel`). */
    event: string;
    /** Time-lane label for a row placed on its day by its deadline. */
    due: string;
    /** Beside a timed row later today: how long until it starts,
     *  `span` already formatted ("2h 15m"). */
    startsIn: (span: string) => string;
  };
  sidePanel: {
    /** Accessible name of the desktop side panel. */
    title: string;
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
    toggleCancelled: string;
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
    /** Advanced section: one group per open workspace with its sync facts. */
    advanced: string;
    syncStatus: string;
    seq: string;
    epoch: string;
    fingerprint: string;
    contents: string;
    workspaceId: string;
    language: string;
    languageSpanish: string;
    languageEnglish: string;
    theme: string;
    auto: string;
    light: string;
    dark: string;
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
    deviceActions: string;
    renameDevice: string;
    revoke: string;
    revoking: string;
    revokeDeviceConfirm: (name: string) => string;
    failedToRenameDevice: string;
    failedToRevokeDevice: string;
    failedToLoadDevices: string;
  };
  /** `SharingDialog` (members, invite links) and `InviteDialog` (the
   *  receiving end of a link); `spec/sharing-plan.md` §Invites. */
  sharing: {
    title: (name: string) => string;
    description: string;
    members: string;
    you: string;
    owner: string;
    inviteLink: string;
    linkHint: string;
    createLink: string;
    creating: string;
    copied: string;
    pendingLinks: string;
    createdAgo: (rel: string) => string;
    revoke: string;
    ownerOnly: string;
    loadFailed: string;
    createFailed: string;
    revokeFailed: string;
    memberMenu: string;
    removeMember: string;
    removeMemberTitle: string;
    removeMemberConfirm: (email: string) => string;
    removeMemberFailed: string;
    lastOwner: string;
    removedNotice: (name: string) => string;
    /** Avatar tooltip when the creator is no longer a member. */
    formerMember: string;
    inviteTitle: string;
    inviteFrom: (hint: string) => string;
    inviteSignIn: string;
    join: string;
    joining: string;
    signIn: string;
    notNow: string;
    inviteError: { invalid: string; gone: string; failed: string };
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
      resetPassword: "Restablecer contraseña",
      newPassword: "Nueva contraseña",
      recoveryCode: "Código de recuperación (12 palabras)",
      forgotPassword: "¿Olvidaste tu contraseña? Usa tu código de recuperación",
      backToSignIn: "Volver a iniciar sesión",
      recoveryCodeInvalid: "Eso no es un código de recuperación válido de 12 palabras",
      recoveryNotEnrolled: "Esta cuenta no tiene código de recuperación",
      signupRecoveryNote:
        "Recibirás un código de recuperación de 12 palabras. Es la única forma de volver a entrar si olvidas tu contraseña: Monoplan no puede restablecerla por ti.",
      recoveryTitle: "Tu código de recuperación",
      recoveryIntro:
        "Anota estas 12 palabras y guárdalas en un lugar seguro. Solo se muestran una vez y son la única forma de restablecer una contraseña olvidada.",
      recoverySaved: "He anotado estas 12 palabras",
      copied: "Copiado",
      continue: "Continuar",
      prereleaseNotice:
        "Esta es una versión preliminar. Los datos se borrarán con regularidad.",
      prereleaseLink:
        "Suscríbete a nuestra lista de correo para saber cuándo se lance Monoplan.",
    },
    nav: {
      inbox: "Bandeja de entrada",
      events: "Eventos",
      focus: "Destacados",
      upcoming: "Calendario",
      done: "Hecho",
      bin: "Papelera",
      archiveList: "Archivar",
      archived: "Archivada",
      unarchiveList: "Restaurar",
      renameList: "Renombrar",
      newList: "Nueva lista",
      personal: "Personal",
      untitledDoc: "Espacio sin nombre",
      newDoc: "Nuevo espacio",
      newDocName: "Nombre del espacio",
      createDoc: "Crear",
      renameDoc: "Renombrar espacio",
      docMenu: "Menú del espacio",
      deleteDoc: "Eliminar espacio",
      deleteDocConfirm: (name) =>
        `¿Eliminar «${name}»? Sus listas y elementos se eliminarán de todos los dispositivos y quienes lo compartan perderán el acceso. Esta acción no se puede deshacer.`,
      deleteDocTypeName: (name) => `Escribe «${name}» para confirmar.`,
      deleteDocFailed: "No se pudo eliminar el espacio",
      createDocFailed: "No se pudo crear el espacio",
      sharing: "Compartir…",
      connected: "Conectado",
      disconnected: "Desconectado",
      offline: "Sin conexión",
      synced: "Sincronizado",
      syncing: "Sincronizando",
      lastSynced: (rel) => `Sincronizado ${rel}`,
      seqLabel: (n) => `seq #${n}`,
      epochLabel: (n) => `epoch ${n}`,
      itemsListsCount: (items, lists) =>
        `${items} elemento${items === 1 ? "" : "s"}, ${lists} lista${lists === 1 ? "" : "s"}`,
      docsCount: (n) => `${n} espacio${n === 1 ? "" : "s"}`,
      undo: "Deshacer",
      redo: "Rehacer",
      settings: "Ajustes",
      website: "Sitio web de Monoplan",
      logOut: "Cerrar sesión",
      logOutTitle: "¿Cerrar sesión en Monoplan?",
      logOutConfirm:
        "Esto elimina la copia local de tus datos de este navegador, incluidos los cambios que aún no se han sincronizado. Lo que ya está sincronizado permanece en el servidor y vuelve cuando inicias sesión de nuevo.",
      exportJson: "Exportar JSON",
      exportFailed: "No se pudo exportar",
      importJson: "Importar JSON",
      importSucceeded: (items, lists) =>
        `Importado: ${items} elemento${items === 1 ? "" : "s"}, ${lists} lista${lists === 1 ? "" : "s"}`,
      importFailed: "No se pudo importar el archivo",
    },
    workspace: {
      emptyBin: "Vaciar papelera",
      emptyBinTitle: "Vaciar papelera",
      emptyBinConfirm: "¿Seguro que quieres borrar permanentemente los elementos de la papelera?",
      createWithSpace: "Pulsa Espacio para crear un elemento nuevo",
      emptyState: "Nada aquí.",
      notes: "Notas",
      hasNotes: "Tiene notas",
      markDone: "Marcar como hecho",
      markNotDone: "Marcar como no hecho",
      markCancelled: "Cancelar",
      reopen: "Reabrir",
      createdStamp: (when) => `Creado ${when}`,
      createdBy: "por",
      activity: "Actividad",
      activityCompleted: (when) => `Completado ${when}`,
      activityElapsed: (span) => `Tiempo transcurrido: ${span}`,
      activityCancelled: (when) => `Cancelado ${when}`,
      duplicate: "Duplicar",
      moveToBin: "Mover a la papelera",
      moveToList: "Mover a la lista",
      changeStatus: "Cambiar estado",
      status: "Estado",
      searchLists: (workspace) =>
        workspace ? `Buscar listas en ${workspace}` : "Buscar listas",
      moveItem: "Mover elemento",
      moveItems: "Mover elementos",
      noMatchingLists: "No hay listas coincidentes",
      currentList: "Actual",
      displayOptions: "Opciones de visualización",
      showOriginList: "Mostrar lista",
      showState: "Mostrar estado",
      allDocs: "Todos los espacios",
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
      cancelledLane: "Cancelado",
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
      add: "Añadir fecha límite",
      unset: "Fecha límite",
      overdue: "Vencido",
      today: "Hoy",
      tomorrow: "Mañana",
      clear: "Borrar",
      remove: "Quitar fecha límite",
      setDate: "Elegir fecha límite…",
      prevMonth: "Mes anterior",
      nextMonth: "Mes siguiente",
    },
    when: {
      label: "Fecha",
      placeholder: "Fecha",
      allDay: "Todo el día",
      today: "Hoy",
      tomorrow: "Mañana",
      remove: "Quitar fecha",
      setDate: "Elegir fecha…",
      time: "Hora",
      clearTime: "Quitar hora",
      end: "Fin",
      endDate: "Fecha de fin",
    },
    place: {
      label: "Lugar",
      placeholder: "Lugar",
      remove: "Quitar lugar",
      edit: "Editar lugar",
      asTyped: "Tal como está escrito",
      searching: "Buscando…",
      noResults: "Sin resultados",
      error: "No se pudo buscar",
      known: "Usado antes",
      openMap: "Abrir mapa",
      credit: "© Colaboradores de OpenStreetMap",
    },
    upcoming: {
      emptyToday: "Nada para hoy",
      docs: "Espacios",
      unscheduled: "Sin fecha",
      event: "Evento",
      due: "Vence",
      startsIn: (span) => `en ${span}`,
    },
    sidePanel: {
      title: "Barra de contexto",
      selectedCount: (n) => `${n} elementos seleccionados`,
      selectionActions: "Acciones sobre la selección",
      clearSelection: "Deseleccionar",
    },
    shortcuts: {
      title: "Atajos de teclado",
      newItem: "Nuevo elemento",
      openItem: "Abrir elemento",
      toggleDone: "Marcar como hecho",
      toggleCancelled: "Cancelar / reabrir",
      toggleFocus: "Destacar o quitar de Destacados",
      moveToList: "Mover a la lista",
      duplicate: "Duplicar",
      copy: "Copiar",
      undo: "Deshacer",
      redo: "Rehacer",
      bin: "Mover a la papelera",
      switchList: "Cambiar de vista",
      switchLane: "Cambiar de carril",
      goToView: "Ir a Destacados / Próximo / Hecho / Entrada",
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
      add: "Destacar",
      remove: "Quitar de Destacados",
      badge: "Destacado",
      showInList: (list: string) => `Ver en ${list}`,
      showInFocus: "Ver en Destacados",
      empty:
        "Destacados está vacío. Añade un elemento nuevo aquí, o haz clic derecho en uno existente y elige «Destacar», para organizar en qué estás trabajando.",
    },
    settings: {
      general: "General",
      account: "Cuenta",
      devices: "Dispositivos",
      advanced: "Avanzado",
      syncStatus: "Estado",
      seq: "Secuencia",
      epoch: "Epoch",
      fingerprint: "Huella",
      contents: "Contenido",
      workspaceId: "ID del espacio",
      language: "Idioma",
      languageSpanish: "Español",
      languageEnglish: "English",
      theme: "Tema",
      auto: "Auto",
      light: "Claro",
      dark: "Oscuro",
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
    sharing: {
      title: (name) => `Compartir «${name}»`,
      description:
        "Quien se une ve todo este espacio, incluido su historial.",
      members: "Miembros",
      you: "Tú",
      owner: "Propietario",
      inviteLink: "Enlace de invitación",
      linkHint: "De un solo uso y caduca en 7 días.",
      createLink: "Crear enlace de invitación",
      creating: "Creando…",
      copied: "Copiado",
      pendingLinks: "Enlaces sin usar",
      createdAgo: (rel) => `Creado ${rel}`,
      revoke: "Revocar",
      ownerOnly: "Solo el propietario del espacio puede invitar.",
      loadFailed: "No se pudieron cargar los detalles",
      createFailed: "No se pudo crear el enlace",
      revokeFailed: "No se pudo revocar el enlace",
      memberMenu: "Opciones del miembro",
      removeMember: "Quitar usuario",
      removeMemberTitle: "¿Quitar usuario?",
      removeMemberConfirm: (email) =>
        `${email} dejará de recibir cambios de este espacio. Lo que ya haya sincronizado sigue en sus dispositivos.`,
      removeMemberFailed: "No se pudo quitar al usuario",
      lastOwner: "El espacio necesita al menos un propietario.",
      removedNotice: (name) => `Te han quitado del espacio «${name}». Se ha eliminado de este dispositivo.`,
      formerMember: "Un antiguo miembro",
      inviteTitle: "Te han invitado",
      inviteFrom: (hint) => `${hint} te invita a un espacio compartido.`,
      inviteSignIn: "Inicia sesión o crea una cuenta para unirte.",
      join: "Unirme",
      joining: "Uniéndote…",
      signIn: "Iniciar sesión",
      notNow: "Ahora no",
      inviteError: {
        invalid: "Este enlace de invitación no es válido.",
        gone: "Este enlace de invitación ha caducado o ya se ha usado.",
        failed: "No se pudo unir al espacio.",
      },
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
      resetPassword: "Reset password",
      newPassword: "New password",
      recoveryCode: "Recovery code (12 words)",
      forgotPassword: "Forgot your password? Use your recovery code",
      backToSignIn: "Back to sign in",
      recoveryCodeInvalid: "That is not a valid 12-word recovery code",
      recoveryNotEnrolled: "This account has no recovery code",
      signupRecoveryNote:
        "You will get a 12-word recovery code. It is the only way back in if you forget your password: Monoplan cannot reset it for you.",
      recoveryTitle: "Your recovery code",
      recoveryIntro:
        "Write these 12 words down and keep them somewhere safe. They are shown only once and are the only way to reset a forgotten password.",
      recoverySaved: "I have written down these 12 words",
      copied: "Copied",
      continue: "Continue",
      prereleaseNotice:
        "This is a pre-release build. Data will be regularly deleted.",
      prereleaseLink:
        "Sign up to the mailing list to get notified when Monoplan releases.",
    },
    nav: {
      inbox: "Inbox",
      events: "Events",
      focus: "Starred",
      upcoming: "Calendar",
      done: "Done",
      bin: "Bin",
      archiveList: "Archive",
      archived: "Archived",
      unarchiveList: "Unarchive",
      renameList: "Rename",
      newList: "New list",
      personal: "Personal",
      untitledDoc: "Untitled workspace",
      newDoc: "New workspace",
      newDocName: "Workspace name",
      createDoc: "Create",
      renameDoc: "Rename workspace",
      docMenu: "Workspace menu",
      deleteDoc: "Delete workspace",
      deleteDocConfirm: (name) =>
        `Delete “${name}”? Its lists and items are removed from every device, and anyone it is shared with loses access. This cannot be undone.`,
      deleteDocTypeName: (name) => `Type “${name}” to confirm.`,
      deleteDocFailed: "Could not delete workspace",
      createDocFailed: "Could not create workspace",
      sharing: "Sharing…",
      connected: "Connected",
      disconnected: "Disconnected",
      offline: "Offline",
      synced: "Synced",
      syncing: "Syncing",
      lastSynced: (rel) => `Synced ${rel}`,
      seqLabel: (n) => `seq #${n}`,
      epochLabel: (n) => `epoch ${n}`,
      itemsListsCount: (items, lists) =>
        `${items} item${items === 1 ? "" : "s"}, ${lists} list${lists === 1 ? "" : "s"}`,
      docsCount: (n) => `${n} workspace${n === 1 ? "" : "s"}`,
      undo: "Undo",
      redo: "Redo",
      settings: "Settings",
      website: "Monoplan website",
      logOut: "Log out",
      logOutTitle: "Sign out of Monoplan?",
      logOutConfirm:
        "This removes the local copy of your data from this browser, including any changes that haven't synced yet. Everything already synced stays on the server and comes back when you sign in again.",
      exportJson: "Export JSON",
      exportFailed: "Could not export",
      importJson: "Import JSON",
      importSucceeded: (items, lists) =>
        `Imported ${items} item${items === 1 ? "" : "s"}, ${lists} list${lists === 1 ? "" : "s"}`,
      importFailed: "Could not import file",
    },
    workspace: {
      emptyBin: "Empty",
      emptyBinTitle: "Empty bin",
      emptyBinConfirm: "Are you sure you want to permanently erase items in the bin?",
      createWithSpace: "Press Space to create a new item",
      emptyState: "Nothing here.",
      notes: "Notes",
      hasNotes: "Has notes",
      markDone: "Mark as done",
      markNotDone: "Mark as not done",
      markCancelled: "Cancel",
      reopen: "Reopen",
      createdStamp: (when) => `Created ${when}`,
      createdBy: "by",
      activity: "Activity",
      activityCompleted: (when) => `Completed ${when}`,
      activityElapsed: (span) => `${span} elapsed`,
      activityCancelled: (when) => `Cancelled ${when}`,
      duplicate: "Duplicate",
      moveToBin: "Move to bin",
      moveToList: "Move to list",
      changeStatus: "Change status",
      status: "Status",
      searchLists: (workspace) =>
        workspace ? `Search lists in ${workspace}` : "Search lists",
      moveItem: "Move item",
      moveItems: "Move items",
      noMatchingLists: "No matching lists",
      currentList: "Current",
      displayOptions: "Display options",
      showOriginList: "Show list",
      showState: "Show state",
      allDocs: "All workspaces",
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
      cancelledLane: "Cancelled",
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
      add: "Add deadline",
      unset: "Deadline",
      overdue: "Overdue",
      today: "Today",
      tomorrow: "Tomorrow",
      clear: "Clear",
      remove: "Remove deadline",
      setDate: "Set deadline…",
      prevMonth: "Previous month",
      nextMonth: "Next month",
    },
    when: {
      label: "Date",
      placeholder: "Date",
      allDay: "All day",
      today: "Today",
      tomorrow: "Tomorrow",
      remove: "Remove date",
      setDate: "Set date…",
      time: "Time",
      clearTime: "Remove time",
      end: "End",
      endDate: "End date",
    },
    place: {
      label: "Place",
      placeholder: "Place",
      remove: "Remove place",
      edit: "Edit place",
      asTyped: "As typed",
      searching: "Searching…",
      noResults: "No results",
      error: "Lookup failed",
      known: "Used before",
      openMap: "Open map",
      credit: "© OpenStreetMap contributors",
    },
    upcoming: {
      emptyToday: "Nothing due today",
      docs: "Workspaces",
      unscheduled: "Unscheduled",
      event: "Event",
      due: "Due",
      startsIn: (span) => `in ${span}`,
    },
    sidePanel: {
      title: "Context sidebar",
      selectedCount: (n) => `${n} items selected`,
      selectionActions: "Selection actions",
      clearSelection: "Clear selection",
    },
    shortcuts: {
      title: "Keyboard shortcuts",
      newItem: "New item",
      openItem: "Open item",
      toggleDone: "Toggle done",
      toggleCancelled: "Cancel / reopen",
      toggleFocus: "Toggle star",
      moveToList: "Move to list",
      duplicate: "Duplicate",
      copy: "Copy",
      undo: "Undo",
      redo: "Redo",
      bin: "Move to bin",
      switchList: "Switch view",
      switchLane: "Switch lane",
      goToView: "Go to Starred / Upcoming / Done / Inbox",
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
      add: "Star",
      remove: "Unstar",
      badge: "Starred",
      showInList: (list: string) => `Show in ${list}`,
      showInFocus: "Show in Starred",
      empty: "Nothing starred yet.",
    },
    settings: {
      general: "General",
      account: "Account",
      devices: "Devices",
      advanced: "Advanced",
      syncStatus: "Status",
      seq: "Sequence",
      epoch: "Epoch",
      fingerprint: "Fingerprint",
      contents: "Contents",
      workspaceId: "Workspace id",
      language: "Language",
      languageSpanish: "Español",
      languageEnglish: "English",
      theme: "Theme",
      auto: "Auto",
      light: "Light",
      dark: "Dark",
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
    sharing: {
      title: (name) => `Share “${name}”`,
      description:
        "Anyone who joins can read everything in this workspace, including its history.",
      members: "Members",
      you: "You",
      owner: "Owner",
      inviteLink: "Invite link",
      linkHint: "Single use, expires in 7 days.",
      createLink: "Create invite link",
      creating: "Creating…",
      copied: "Copied",
      pendingLinks: "Unused links",
      createdAgo: (rel) => `Created ${rel}`,
      revoke: "Revoke",
      ownerOnly: "Only the workspace owner can invite people.",
      loadFailed: "Could not load sharing details",
      createFailed: "Could not create the link",
      revokeFailed: "Could not revoke the link",
      memberMenu: "Member options",
      removeMember: "Remove user",
      removeMemberTitle: "Remove user?",
      removeMemberConfirm: (email) =>
        `${email} will stop receiving changes to this workspace. Anything they already synced stays on their devices.`,
      removeMemberFailed: "Could not remove the user",
      lastOwner: "A workspace needs at least one owner.",
      removedNotice: (name) => `You were removed from “${name}”. It has been removed from this device.`,
      formerMember: "A former member",
      inviteTitle: "You're invited",
      inviteFrom: (hint) => `${hint} has invited you to a shared workspace.`,
      inviteSignIn: "Sign in or create an account to join.",
      join: "Join",
      joining: "Joining…",
      signIn: "Sign in",
      notNow: "Not now",
      inviteError: {
        invalid: "This invite link isn't valid.",
        gone: "This invite link has expired or was already used.",
        failed: "Could not join the workspace.",
      },
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
/** Status label for an item's state in the status controls. A task takes
 *  the lane label. An event (`spec/events-plan.md`) has no lanes, so its
 *  four open states read as one, "Event"; Done and Cancelled read as they
 *  do on a task. Display only: the stored state is untouched. */
export function statusLabel(
  m: Messages,
  state: WorkflowState,
  event: boolean,
): string {
  return event && state !== "done" && state !== "cancelled"
    ? m.upcoming.event
    : laneLabel(m, state);
}

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
    case "cancelled":
      return m.board.cancelledLane;
  }
}
