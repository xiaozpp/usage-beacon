import { Dashboard } from "./components/Dashboard";
import { I18nProvider } from "./lib/i18n";

function App() {
  return (
    <I18nProvider>
      <Dashboard />
    </I18nProvider>
  );
}

export default App;
